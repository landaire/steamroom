use clap::Parser;
use prost::Message;
use std::collections::HashMap;
use std::path::PathBuf;

mod renderer;

#[derive(Parser)]
#[command(
    name = "proto-extract",
    about = "Extract protobuf definitions from PE binaries"
)]
struct Args {
    #[arg(long)]
    dll: PathBuf,

    #[arg(long, default_value = "proto_out")]
    output: PathBuf,

    #[arg(long)]
    filter: Option<String>,

    #[arg(long)]
    binary_out: Option<PathBuf>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    eprintln!("Loading PE binary: {}", args.dll.display());
    let data = std::fs::read(&args.dll)?;

    let descriptors = extract_descriptors(&data)?;
    eprintln!("Found {} FileDescriptorProto blobs", descriptors.len());

    if let Some(ref bin_path) = args.binary_out {
        let fds = prost_types::FileDescriptorSet {
            file: descriptors.clone(),
        };
        let encoded = fds.encode_to_vec();
        std::fs::write(bin_path, &encoded)?;
        eprintln!("Wrote binary FileDescriptorSet to {}", bin_path.display());
    }

    std::fs::create_dir_all(&args.output)?;

    let mut written = 0;
    for desc in &descriptors {
        let name = desc.name.as_deref().unwrap_or("unknown");
        if let Some(ref filter) = args.filter {
            let patterns: Vec<&str> = filter.split(',').collect();
            if !patterns.iter().any(|p| name.contains(p)) {
                continue;
            }
        }
        let proto_text = renderer::render_proto(desc);
        let out_path = args.output.join(name);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out_path, &proto_text)?;
        eprintln!("  Wrote {}", out_path.display());
        written += 1;
    }

    eprintln!("Wrote {written} proto files");
    Ok(())
}

#[allow(dead_code)]
struct SectionInfo {
    file_offset: usize,
    virtual_address: u64,
    data: Vec<u8>,
}

fn extract_descriptors(
    pe_data: &[u8],
) -> Result<Vec<prost_types::FileDescriptorProto>, Box<dyn std::error::Error>> {
    use object::read::pe::PeFile64;
    use object::Object;
    use object::ObjectSection;

    let pe = PeFile64::parse(pe_data)?;
    let _image_base = pe.relative_address_base();

    let mut rdata_sections = Vec::new();
    let mut data_section: Option<SectionInfo> = None;

    for section in pe.sections() {
        let name = section.name()?;
        let sec_data = section.data()?.to_vec();
        let va = section.address();
        let file_off = section
            .file_range()
            .map(|(off, _)| off as usize)
            .unwrap_or(0);

        if name == ".rdata" {
            rdata_sections.push(SectionInfo {
                file_offset: file_off,
                virtual_address: va,
                data: sec_data,
            });
        } else if name == ".data" {
            data_section = Some(SectionInfo {
                file_offset: file_off,
                virtual_address: va,
                data: sec_data,
            });
        }
    }

    // Phase 1: Scan .rdata for candidate blob starts (0x0a + varint + ".proto" suffix)
    let mut candidates: Vec<(u64, String)> = Vec::new(); // (virtual_addr, proto_name)

    for sec in &rdata_sections {
        let sec_data = &sec.data;
        let mut offset = 0;
        while offset < sec_data.len().saturating_sub(4) {
            if sec_data[offset] == 0x0a {
                if let Some(name) = check_proto_name(sec_data, offset) {
                    let va = sec.virtual_address + offset as u64;
                    candidates.push((va, name));
                }
            }
            offset += 1;
        }
    }

    eprintln!(
        "Phase 1: Found {} candidate .proto name references",
        candidates.len()
    );

    // Phase 2: For each candidate, find its size from the .data section
    // The .data section contains registration entries with (size: u32, ptr: u64) pairs
    let blob_sizes = if let Some(ref ds) = data_section {
        find_blob_sizes(ds, &candidates)
    } else {
        HashMap::new()
    };

    eprintln!(
        "Phase 2: Found sizes for {} blobs via .data cross-reference",
        blob_sizes.len()
    );

    // Phase 3: Decode each candidate
    let mut descriptors = Vec::new();
    let mut seen_names = std::collections::HashSet::new();

    for (va, name) in &candidates {
        if seen_names.contains(name.as_str()) {
            continue;
        }

        // Find the section data for this VA
        let Some((sec_data, sec_offset)) = find_va_in_sections(&rdata_sections, *va) else {
            continue;
        };

        let remaining = &sec_data[sec_offset..];

        // Try known size first
        let desc = if let Some(&size) = blob_sizes.get(va) {
            let end = (size as usize).min(remaining.len());
            prost_types::FileDescriptorProto::decode(&remaining[..end]).ok()
        } else {
            None
        };

        // Fallback: try progressive sizes
        let desc = desc.or_else(|| {
            for try_len in [128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536] {
                let end = try_len.min(remaining.len());
                if let Ok(d) = prost_types::FileDescriptorProto::decode(&remaining[..end]) {
                    if d.name.as_deref() == Some(name.as_str()) && has_content(&d) {
                        // Verify: re-encode length should be <= our window
                        if d.encoded_len() <= end {
                            return Some(d);
                        }
                    }
                }
            }
            None
        });

        if let Some(desc) = desc {
            if desc.name.as_deref() == Some(name.as_str()) && has_content(&desc) {
                seen_names.insert(name.clone());
                descriptors.push(desc);
            }
        }
    }

    // Sort by name for deterministic output
    descriptors.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(descriptors)
}

fn check_proto_name(data: &[u8], offset: usize) -> Option<String> {
    let remaining = &data[offset..];
    if remaining.len() < 3 {
        return None;
    }

    let (name_len, varint_len) = decode_varint(&remaining[1..])?;
    if name_len == 0 || name_len > 120 {
        return None;
    }

    let name_start = 1 + varint_len;
    let name_end = name_start + name_len as usize;
    if remaining.len() < name_end {
        return None;
    }

    let name_bytes = &remaining[name_start..name_end];
    let name = std::str::from_utf8(name_bytes).ok()?;

    if !name.ends_with(".proto") {
        return None;
    }

    // Basic sanity: name should be printable ASCII, no weird chars
    if !name.bytes().all(|b| b.is_ascii_graphic() || b == b'/') {
        return None;
    }

    Some(name.to_string())
}

fn find_blob_sizes(data_section: &SectionInfo, candidates: &[(u64, String)]) -> HashMap<u64, u32> {
    let mut sizes = HashMap::new();
    let ds = &data_section.data;

    // Build a set of candidate VAs for fast lookup
    let candidate_vas: std::collections::HashSet<u64> =
        candidates.iter().map(|(va, _)| *va).collect();

    // Scan .data for 8-byte pointers matching candidate VAs
    // Registration entries typically have: ... size(4 bytes) ptr(8 bytes) ...
    if ds.len() < 12 {
        return sizes;
    }

    for offset in (0..ds.len() - 8).step_by(4) {
        let ptr = u64::from_le_bytes(ds[offset..offset + 8].try_into().unwrap());
        if candidate_vas.contains(&ptr) {
            // Check for size in the 4 bytes before the pointer
            if offset >= 4 {
                let size = u32::from_le_bytes(ds[offset - 4..offset].try_into().unwrap());
                if (64..131072).contains(&size) {
                    sizes.insert(ptr, size);
                }
            }
        }
    }

    sizes
}

fn find_va_in_sections(sections: &[SectionInfo], va: u64) -> Option<(&[u8], usize)> {
    for sec in sections {
        if va >= sec.virtual_address {
            let offset = (va - sec.virtual_address) as usize;
            if offset < sec.data.len() {
                return Some((&sec.data, offset));
            }
        }
    }
    None
}

fn has_content(desc: &prost_types::FileDescriptorProto) -> bool {
    !desc.message_type.is_empty()
        || !desc.enum_type.is_empty()
        || !desc.service.is_empty()
        || !desc.dependency.is_empty()
        || !desc.extension.is_empty()
}

fn decode_varint(data: &[u8]) -> Option<(u64, usize)> {
    let mut value: u64 = 0;
    let mut shift = 0;
    for (i, &byte) in data.iter().enumerate() {
        value |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some((value, i + 1));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}
