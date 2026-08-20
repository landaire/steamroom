use std::io;
use std::io::Read;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() {
        eprintln!("usage: kv2json <file>...");
        eprintln!("       kv2json -       (read stdin)");
        std::process::exit(1);
    }

    let mut exit_code = 0;

    for arg in &args {
        let (name, data) = if arg == "-" {
            let mut buf = Vec::new();
            io::stdin().read_to_end(&mut buf).unwrap();
            ("<stdin>".to_string(), buf)
        } else {
            let path = PathBuf::from(arg);
            match std::fs::read(&path) {
                Ok(data) => (arg.clone(), data),
                Err(e) => {
                    eprintln!("{arg}: {e}");
                    exit_code = 1;
                    continue;
                }
            }
        };

        match convert(&data) {
            Ok(json) => {
                if args.len() > 1 {
                    println!("// {name}");
                }
                println!("{json}");
            }
            Err(e) => {
                eprintln!("{name}: {e}");
                exit_code = 1;
            }
        }
    }

    std::process::exit(exit_code);
}

fn convert(data: &[u8]) -> Result<String, String> {
    // Try binary first (starts with a KV tag byte 0x00..=0x0b)
    if !data.is_empty() && data[0] <= 0x0b {
        let kv =
            steamroom::types::KeyValue::from_binary(data).map_err(|e| format!("binary KV: {e}"))?;
        return serde_json::to_string_pretty(&kv).map_err(|e| format!("json: {e}"));
    }

    // Text KV: strip BOM, wrap in a synthetic root so multiple top-level
    // pairs (common in Steam localization files) are captured.
    let text = strip_bom(data);
    let s = std::str::from_utf8(text).map_err(|e| format!("invalid UTF-8: {e}"))?;

    let wrapped = format!("\"root\"\n{{\n{s}\n}}");
    let kv =
        steamroom::types::KeyValue::from_text(&wrapped).map_err(|e| format!("text KV: {e}"))?;
    serde_json::to_string_pretty(&kv).map_err(|e| format!("json: {e}"))
}

fn strip_bom(data: &[u8]) -> &[u8] {
    if data.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &data[3..]
    } else {
        data
    }
}
