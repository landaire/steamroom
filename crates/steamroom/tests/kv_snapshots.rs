use steamroom::types::key_value::parse_binary_kv;
use steamroom::types::key_value::parse_text_kv;

#[test]
fn text_kv_app_state() {
    let input = r#""AppState"
{
    "appid"     "480"
    "name"      "Spacewar"
    "Universe"  "1"
    "installdir"    "Spacewar"
    "StateFlags"    "4"
}"#;
    let kv = parse_text_kv(input).unwrap();
    insta::assert_toml_snapshot!(kv);
}

#[test]
fn text_kv_nested() {
    let input = r#""root"
{
    "depots"
    {
        "481"
        {
            "manifests"
            {
                "public"
                {
                    "gid"   "3183503801510301321"
                }
            }
        }
    }
}"#;
    let kv = parse_text_kv(input).unwrap();
    insta::assert_toml_snapshot!(kv);
}

#[test]
fn text_kv_escaped_strings() {
    let input = r#""test"
{
    "path"  "C:\\Program Files\\Steam"
    "quote" "say \"hello\""
}"#;
    let kv = parse_text_kv(input).unwrap();
    insta::assert_toml_snapshot!(kv);
}

#[test]
fn binary_kv_roundtrip() {
    let mut data = Vec::new();
    data.push(0u8);
    data.extend_from_slice(b"AppState\0");
    data.push(1u8);
    data.extend_from_slice(b"appid\0");
    data.extend_from_slice(b"480\0");
    data.push(2u8);
    data.extend_from_slice(b"buildid\0");
    data.extend_from_slice(&3538192u32.to_le_bytes());
    data.push(7u8);
    data.extend_from_slice(b"SizeOnDisk\0");
    data.extend_from_slice(&1906688u64.to_le_bytes());
    data.push(8u8);

    let kv = parse_binary_kv(&data).unwrap();
    insta::assert_toml_snapshot!(kv);
}
