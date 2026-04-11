use prost_types::field_descriptor_proto::Label;
use prost_types::field_descriptor_proto::Type;
use prost_types::DescriptorProto;
use prost_types::EnumDescriptorProto;
use prost_types::FieldDescriptorProto;
use prost_types::FileDescriptorProto;
use prost_types::MethodDescriptorProto;
use prost_types::OneofDescriptorProto;
use prost_types::ServiceDescriptorProto;
use std::fmt::Write;

pub fn render_proto(desc: &FileDescriptorProto) -> String {
    let mut out = String::new();

    let syntax = desc.syntax.as_deref().unwrap_or("proto2");
    writeln!(out, "syntax = \"{syntax}\";").unwrap();
    out.push('\n');

    if let Some(ref pkg) = desc.package {
        writeln!(out, "package {pkg};").unwrap();
        out.push('\n');
    }

    for dep in &desc.dependency {
        writeln!(out, "import \"{dep}\";").unwrap();
    }
    if !desc.dependency.is_empty() {
        out.push('\n');
    }

    if let Some(ref opts) = desc.options {
        render_file_options(&mut out, opts);
    }

    for enum_type in &desc.enum_type {
        render_enum(&mut out, enum_type, 0);
        out.push('\n');
    }

    for msg in &desc.message_type {
        render_message(&mut out, msg, 0);
        out.push('\n');
    }

    for ext in &desc.extension {
        render_extension(&mut out, ext, 0);
    }
    if !desc.extension.is_empty() {
        out.push('\n');
    }

    for svc in &desc.service {
        render_service(&mut out, svc);
        out.push('\n');
    }

    out
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn render_file_options(out: &mut String, opts: &prost_types::FileOptions) {
    if let Some(ref pkg) = opts.java_package {
        writeln!(out, "option java_package = \"{pkg}\";").unwrap();
    }
    if let Some(ref cls) = opts.java_outer_classname {
        writeln!(out, "option java_outer_classname = \"{cls}\";").unwrap();
    }
    if let Some(v) = opts.java_multiple_files {
        writeln!(out, "option java_multiple_files = {v};").unwrap();
    }
    if let Some(v) = opts.optimize_for {
        let name = match v {
            1 => "SPEED",
            2 => "CODE_SIZE",
            3 => "LITE_RUNTIME",
            _ => return,
        };
        writeln!(out, "option optimize_for = {name};").unwrap();
    }
    if let Some(ref ns) = opts.csharp_namespace {
        writeln!(out, "option csharp_namespace = \"{ns}\";").unwrap();
    }
    if let Some(ref prefix) = opts.objc_class_prefix {
        writeln!(out, "option objc_class_prefix = \"{prefix}\";").unwrap();
    }
    if let Some(ref pkg) = opts.go_package {
        writeln!(out, "option go_package = \"{pkg}\";").unwrap();
    }
    if let Some(v) = opts.cc_generic_services {
        writeln!(out, "option cc_generic_services = {v};").unwrap();
    }
    out.push('\n');
}

fn render_message(out: &mut String, msg: &DescriptorProto, depth: usize) {
    let name = msg.name.as_deref().unwrap_or("Unknown");
    indent(out, depth);
    writeln!(out, "message {name} {{").unwrap();

    for enum_type in &msg.enum_type {
        render_enum(out, enum_type, depth + 1);
    }

    for nested in &msg.nested_type {
        // Skip map entry types — prost-build handles these
        if nested
            .options
            .as_ref()
            .is_some_and(|o| o.map_entry == Some(true))
        {
            continue;
        }
        render_message(out, nested, depth + 1);
    }

    // Collect oneof field indices
    let oneof_fields: Vec<Vec<&FieldDescriptorProto>> = {
        let mut groups: Vec<Vec<&FieldDescriptorProto>> = vec![Vec::new(); msg.oneof_decl.len()];
        for field in &msg.field {
            if let Some(idx) = field.oneof_index {
                if let Some(group) = groups.get_mut(idx as usize) {
                    group.push(field);
                }
            }
        }
        groups
    };

    // Render non-oneof fields and oneof groups
    let mut rendered_oneofs = vec![false; msg.oneof_decl.len()];
    for field in &msg.field {
        if let Some(idx) = field.oneof_index {
            let idx = idx as usize;
            if !rendered_oneofs[idx] {
                rendered_oneofs[idx] = true;
                render_oneof(out, &msg.oneof_decl[idx], &oneof_fields[idx], depth + 1);
            }
        } else {
            render_field(out, field, depth + 1);
        }
    }

    for ext in &msg.extension {
        render_extension(out, ext, depth + 1);
    }

    if let Some(_opts) = &msg.options {
        // Extension ranges
    }
    for range in &msg.extension_range {
        indent(out, depth + 1);
        let start = range.start.unwrap_or(0);
        let end = range.end.unwrap_or(536870912);
        if end >= 536870912 {
            writeln!(out, "extensions {start} to max;").unwrap();
        } else {
            writeln!(out, "extensions {start} to {};", end - 1).unwrap();
        }
    }

    indent(out, depth);
    writeln!(out, "}}").unwrap();
}

fn render_field(out: &mut String, field: &FieldDescriptorProto, depth: usize) {
    render_field_inner(out, field, depth, false);
}

fn render_field_in_oneof(out: &mut String, field: &FieldDescriptorProto, depth: usize) {
    render_field_inner(out, field, depth, true);
}

fn render_field_inner(
    out: &mut String,
    field: &FieldDescriptorProto,
    depth: usize,
    in_oneof: bool,
) {
    let name = field.name.as_deref().unwrap_or("unknown");
    let number = field.number.unwrap_or(0);
    let type_name = field_type_name(field);

    indent(out, depth);
    if !in_oneof {
        let label = field_label(field);
        if !label.is_empty() {
            write!(out, "{label} ").unwrap();
        }
    }
    write!(out, "{type_name} {name} = {number}").unwrap();

    let mut options = Vec::new();
    if let Some(ref default) = field.default_value {
        // String and bytes fields need quoted defaults
        let needs_quotes = matches!(field.r#type(), Type::String | Type::Bytes);
        if needs_quotes {
            options.push(format!("default = \"{default}\""));
        } else {
            options.push(format!("default = {default}"));
        }
    }
    if field
        .options
        .as_ref()
        .is_some_and(|o| o.packed == Some(true))
    {
        options.push("packed = true".into());
    }
    if field
        .options
        .as_ref()
        .is_some_and(|o| o.deprecated == Some(true))
    {
        options.push("deprecated = true".into());
    }
    if field.options.as_ref().is_some_and(|o| o.lazy == Some(true)) {
        options.push("lazy = true".into());
    }

    if !options.is_empty() {
        write!(out, " [{}]", options.join(", ")).unwrap();
    }

    writeln!(out, ";").unwrap();
}

fn render_oneof(
    out: &mut String,
    decl: &OneofDescriptorProto,
    fields: &[&FieldDescriptorProto],
    depth: usize,
) {
    let name = decl.name.as_deref().unwrap_or("unknown");
    indent(out, depth);
    writeln!(out, "oneof {name} {{").unwrap();
    for field in fields {
        render_field_in_oneof(out, field, depth + 1);
    }
    indent(out, depth);
    writeln!(out, "}}").unwrap();
}

fn render_enum(out: &mut String, e: &EnumDescriptorProto, depth: usize) {
    let name = e.name.as_deref().unwrap_or("Unknown");
    indent(out, depth);
    writeln!(out, "enum {name} {{").unwrap();

    if e.options
        .as_ref()
        .is_some_and(|o| o.allow_alias == Some(true))
    {
        indent(out, depth + 1);
        writeln!(out, "option allow_alias = true;").unwrap();
    }

    for val in &e.value {
        let vname = val.name.as_deref().unwrap_or("UNKNOWN");
        let vnum = val.number.unwrap_or(0);
        indent(out, depth + 1);
        writeln!(out, "{vname} = {vnum};").unwrap();
    }

    indent(out, depth);
    writeln!(out, "}}").unwrap();
}

fn render_extension(out: &mut String, ext: &FieldDescriptorProto, depth: usize) {
    let extendee = ext.extendee.as_deref().unwrap_or("");
    indent(out, depth);
    writeln!(out, "extend {extendee} {{").unwrap();
    render_field(out, ext, depth + 1);
    indent(out, depth);
    writeln!(out, "}}").unwrap();
}

fn render_service(out: &mut String, svc: &ServiceDescriptorProto) {
    let name = svc.name.as_deref().unwrap_or("Unknown");
    writeln!(out, "service {name} {{").unwrap();

    for method in &svc.method {
        render_method(out, method);
    }

    writeln!(out, "}}").unwrap();
}

fn render_method(out: &mut String, method: &MethodDescriptorProto) {
    let name = method.name.as_deref().unwrap_or("Unknown");
    let input = method.input_type.as_deref().unwrap_or("Unknown");
    let output = method.output_type.as_deref().unwrap_or("Unknown");

    indent(out, 1);
    write!(out, "rpc {name} ({input}) returns ({output})").unwrap();

    // Check for method options
    let has_options = method.options.is_some();
    if has_options {
        writeln!(out, " {{").unwrap();
        // Custom options would go here
        indent(out, 1);
        writeln!(out, "}}").unwrap();
    } else {
        writeln!(out, ";").unwrap();
    }
}

fn field_type_name(field: &FieldDescriptorProto) -> String {
    if let Some(ref type_name) = field.type_name {
        // Strip leading dot from fully-qualified names
        let name = type_name.strip_prefix('.').unwrap_or(type_name);
        return name.to_string();
    }

    match field.r#type() {
        Type::Double => "double",
        Type::Float => "float",
        Type::Int64 => "int64",
        Type::Uint64 => "uint64",
        Type::Int32 => "int32",
        Type::Fixed64 => "fixed64",
        Type::Fixed32 => "fixed32",
        Type::Bool => "bool",
        Type::String => "string",
        Type::Bytes => "bytes",
        Type::Uint32 => "uint32",
        Type::Sfixed32 => "sfixed32",
        Type::Sfixed64 => "sfixed64",
        Type::Sint32 => "sint32",
        Type::Sint64 => "sint64",
        Type::Group => "group",
        Type::Message => "message",
        Type::Enum => "enum",
    }
    .to_string()
}

fn field_label(field: &FieldDescriptorProto) -> &'static str {
    match field.label() {
        Label::Optional => "optional",
        Label::Required => "required",
        Label::Repeated => "repeated",
    }
}
