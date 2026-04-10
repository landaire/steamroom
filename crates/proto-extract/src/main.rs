use clap::Parser;
use prost::Message;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "proto-extract", about = "Extract protobuf definitions from PE binaries")]
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

    std::fs::create_dir_all(&args.output)?;

    for desc in &descriptors {
        let name = desc.name.as_deref().unwrap_or("unknown");
        if let Some(ref filter) = args.filter {
            let patterns: Vec<&str> = filter.split(',').collect();
            if !patterns.iter().any(|p| name.contains(p)) {
                continue;
            }
        }
        let proto_text = render_proto(desc);
        let out_path = args.output.join(name);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out_path, &proto_text)?;
        eprintln!("  Wrote {}", out_path.display());
    }

    Ok(())
}

fn extract_descriptors(
    pe_data: &[u8],
) -> Result<Vec<prost_types::FileDescriptorProto>, Box<dyn std::error::Error>> {
    use object::read::pe::PeFile64;
    use object::Object;
    use object::ObjectSection;

    let pe = PeFile64::parse(pe_data)?;

    let mut descriptors = Vec::new();

    for section in pe.sections() {
        let section_name = section.name()?;
        if section_name != ".rdata" {
            continue;
        }

        let section_data = section.data()?;
        let mut offset = 0;

        while offset < section_data.len().saturating_sub(4) {
            if section_data[offset] == 0x0a {
                if let Some(desc) = try_parse_descriptor(section_data, offset) {
                    descriptors.push(desc);
                }
            }
            offset += 1;
        }
    }

    descriptors.dedup_by(|a, b| a.name == b.name);
    Ok(descriptors)
}

fn try_parse_descriptor(
    data: &[u8],
    offset: usize,
) -> Option<prost_types::FileDescriptorProto> {
    let remaining = &data[offset..];
    if remaining.len() < 3 {
        return None;
    }

    // field 1 (name), wire type 2 (length-delimited) = tag byte 0x0a
    // next byte(s) = varint length of the name string
    let name_len = remaining[1] as usize;
    if name_len == 0 || name_len > 120 {
        return None;
    }

    if remaining.len() < 2 + name_len {
        return None;
    }

    let name_bytes = &remaining[2..2 + name_len];
    let name = std::str::from_utf8(name_bytes).ok()?;

    if !name.ends_with(".proto") {
        return None;
    }

    // Try progressively larger decode windows
    for try_len in [256, 1024, 4096, 16384, 65536] {
        let end = (offset + try_len).min(data.len());
        let slice = &data[offset..end];
        if let Ok(desc) = prost_types::FileDescriptorProto::decode(slice) {
            if desc.name.as_deref() == Some(name) {
                return Some(desc);
            }
        }
    }

    None
}

fn render_proto(_desc: &prost_types::FileDescriptorProto) -> String {
    todo!()
}
