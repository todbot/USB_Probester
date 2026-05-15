use usb_types::{CollectionKind, HidIoFlags, HidNode};

// ── Public API ─────────────────────────────────────────────────────────────────

pub fn parse(bytes: &[u8]) -> Result<Vec<HidNode>, ParseError> {
    let items = read_items(bytes)?;
    build_tree(&items)
}

/// Render parsed nodes as USB Prober-style text.
/// `base_indent` is the column for top-level items (26 in the fixture).
pub fn render_text(nodes: &[HidNode], base_indent: usize) -> String {
    let mut out = String::new();
    render_nodes(nodes, base_indent, 0, &mut out);
    out
}

// ── Error ──────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ParseError {
    UnexpectedEnd { offset: usize },
    UnmatchedEndCollection { offset: usize },
    UnclosedCollections(usize),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::UnexpectedEnd { offset } => write!(f, "unexpected end at offset {offset}"),
            ParseError::UnmatchedEndCollection { offset } => {
                write!(f, "unmatched End Collection at offset {offset}")
            }
            ParseError::UnclosedCollections(n) => write!(f, "{n} unclosed Collection(s)"),
        }
    }
}

impl std::error::Error for ParseError {}

// ── Item stream reader ─────────────────────────────────────────────────────────

#[derive(Debug)]
struct Item {
    itype: IType,
    tag: u8,
    value: i64,
    data_len: u8,
    offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum IType {
    Main,
    Global,
    Local,
}

fn read_items(bytes: &[u8]) -> Result<Vec<Item>, ParseError> {
    let mut items = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let prefix = bytes[i];
        let size_code = prefix & 0x03;
        let itype = match (prefix >> 2) & 0x03 {
            0 => IType::Main,
            1 => IType::Global,
            _ => IType::Local,
        };
        let tag = prefix >> 4;
        let data_len: usize = match size_code {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => 4,
        };
        let offset = i;
        i += 1;
        if i + data_len > bytes.len() {
            return Err(ParseError::UnexpectedEnd { offset: i });
        }
        let raw = &bytes[i..i + data_len];
        i += data_len;

        // Logical/Physical Min/Max (global tags 1-4) and Unit Exponent (tag 5) are signed.
        let is_signed = itype == IType::Global && tag >= 1 && tag <= 5;
        let value: i64 = match (data_len, is_signed) {
            (0, _) => 0,
            (1, true) => raw[0] as i8 as i64,
            (1, false) => raw[0] as i64,
            (2, true) => i16::from_le_bytes([raw[0], raw[1]]) as i64,
            (2, false) => u16::from_le_bytes([raw[0], raw[1]]) as i64,
            (4, true) => i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as i64,
            (4, false) => u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as i64,
            _ => unreachable!(),
        };

        items.push(Item { itype, tag, value, data_len: data_len as u8, offset });
    }
    Ok(items)
}

// ── Tree builder ───────────────────────────────────────────────────────────────

fn build_tree(items: &[Item]) -> Result<Vec<HidNode>, ParseError> {
    // Stack of (collection_kind, nodes). Bottom entry is the root (kind=None).
    let mut stack: Vec<(Option<CollectionKind>, Vec<HidNode>)> = vec![(None, Vec::new())];
    let mut usage_page: u16 = 0;

    for item in items {
        match (item.itype, item.tag) {
            (IType::Main, 10) => {
                // Collection — push a new frame, recording the kind.
                stack.push((Some(collection_kind(item.value as u8)), Vec::new()));
            }
            (IType::Main, 12) => {
                // End Collection — pop and wrap.
                if stack.len() < 2 {
                    return Err(ParseError::UnmatchedEndCollection { offset: item.offset });
                }
                let (kind, children) = stack.pop().unwrap();
                let node = HidNode::Collection { kind: kind.unwrap(), children };
                stack.last_mut().unwrap().1.push(node);
            }
            _ => {
                if let Some(node) = item_to_node(item, &mut usage_page) {
                    stack.last_mut().unwrap().1.push(node);
                }
            }
        }
    }

    if stack.len() > 1 {
        return Err(ParseError::UnclosedCollections(stack.len() - 1));
    }

    Ok(stack.pop().unwrap().1)
}

fn item_to_node(item: &Item, usage_page: &mut u16) -> Option<HidNode> {
    match (item.itype, item.tag) {
        // Global
        (IType::Global, 0) => {
            *usage_page = item.value as u16;
            let known = page_name(*usage_page);
            let name = if known.is_empty() {
                "Vendor defined".to_string()
            } else {
                known.to_string()
            };
            Some(HidNode::UsagePage { page_id: *usage_page, name })
        }
        (IType::Global, 1) => Some(HidNode::LogicalMinimum { value: item.value as i32 }),
        (IType::Global, 2) => Some(HidNode::LogicalMaximum { value: item.value as i32 }),
        (IType::Global, 3) => Some(HidNode::PhysicalMinimum { value: item.value as i32 }),
        (IType::Global, 4) => Some(HidNode::PhysicalMaximum { value: item.value as i32 }),
        (IType::Global, 5) => Some(HidNode::UnitExponent { value: item.value as i32 }),
        (IType::Global, 6) => Some(HidNode::Unit { value: item.value as u32 }),
        (IType::Global, 7) => Some(HidNode::ReportSize { value: item.value as u32 }),
        (IType::Global, 8) => Some(HidNode::ReportId { id: item.value as u32 }),
        (IType::Global, 9) => Some(HidNode::ReportCount { value: item.value as u32 }),
        // Local
        (IType::Local, 0) => {
            let (page, usage) = resolve_usage(item.value as u32, *usage_page, item.data_len);
            Some(HidNode::Usage {
                usage_id: usage as u32,
                name: usage_name(page, usage),
            })
        }
        (IType::Local, 1) => Some(HidNode::UsageMinimum { value: item.value as u32 }),
        (IType::Local, 2) => Some(HidNode::UsageMaximum { value: item.value as u32 }),
        // Main: Input / Output / Feature
        (IType::Main, 8) => Some(HidNode::Input { flags: decode_flags(item.value as u32) }),
        (IType::Main, 9) => Some(HidNode::Output { flags: decode_flags(item.value as u32) }),
        (IType::Main, 11) => Some(HidNode::Feature { flags: decode_flags(item.value as u32) }),
        _ => None,
    }
}

fn collection_kind(v: u8) -> CollectionKind {
    match v {
        0 => CollectionKind::Physical,
        1 => CollectionKind::Application,
        2 => CollectionKind::Logical,
        3 => CollectionKind::Report,
        4 => CollectionKind::NamedArray,
        5 => CollectionKind::UsageSwitch,
        6 => CollectionKind::UsageModifier,
        v => CollectionKind::Vendor { id: v },
    }
}

fn resolve_usage(raw: u32, current_page: u16, data_len: u8) -> (u16, u16) {
    if data_len == 4 {
        ((raw >> 16) as u16, (raw & 0xFFFF) as u16)
    } else {
        (current_page, raw as u16)
    }
}

fn decode_flags(v: u32) -> HidIoFlags {
    HidIoFlags {
        constant: v & (1 << 0) != 0,
        variable: v & (1 << 1) != 0,
        relative: v & (1 << 2) != 0,
        wrap: v & (1 << 3) != 0,
        non_linear: v & (1 << 4) != 0,
        no_preferred: v & (1 << 5) != 0,
        null_state: v & (1 << 6) != 0,
        volatile: v & (1 << 7) != 0,
        buffered_bytes: v & (1 << 8) != 0,
    }
}

// ── Usage table ────────────────────────────────────────────────────────────────
// Minimal set covering the Pico 2 fixture. Expand to full HUT 1.5 later.

fn page_name(page: u16) -> &'static str {
    match page {
        0x01 => "Generic Desktop",
        0x02 => "Simulation Controls",
        0x03 => "VR Controls",
        0x04 => "Sports Controls",
        0x05 => "Game Controls",
        0x06 => "Generic Device Controls",
        0x07 => "Keyboard/Keypad",
        0x08 => "LED",
        0x09 => "Button",
        0x0A => "Ordinal",
        0x0B => "Telephony",
        0x0C => "Consumer",
        0x0D => "Digitizer",
        0x0F => "Physical Interface Device",
        0x10 => "Unicode",
        0x14 => "Alphanumeric Display",
        0x40 => "Medical Instruments",
        0x80 => "Monitor",
        0x81 => "Monitor Enumerated Values",
        0x82 => "VESA Virtual Controls",
        0x84 => "Power Device",
        0x85 => "Battery System",
        0x8C => "Bar Code Scanner",
        0x8D => "Scale",
        0x8E => "Magnetic Stripe Reader",
        0x90 => "Camera Control",
        0x91 => "Arcade",
        _ => "",
    }
}

fn usage_name(page: u16, usage: u16) -> Option<String> {
    let name: &str = match (page, usage) {
        // Generic Desktop (0x01)
        (0x01, 0x01) => "Pointer",
        (0x01, 0x02) => "Mouse",
        (0x01, 0x03) => "Reserved",
        (0x01, 0x04) => "Joystick",
        (0x01, 0x05) => "Game Pad",
        (0x01, 0x06) => "Keyboard",
        (0x01, 0x07) => "Keypad",
        (0x01, 0x08) => "Multi-axis Controller",
        (0x01, 0x30) => "X",
        (0x01, 0x31) => "Y",
        (0x01, 0x32) => "Z",
        (0x01, 0x33) => "Rx",
        (0x01, 0x34) => "Ry",
        (0x01, 0x35) => "Rz",
        (0x01, 0x36) => "Slider",
        (0x01, 0x37) => "Dial",
        (0x01, 0x38) => "Wheel",
        (0x01, 0x39) => "Hat Switch",
        (0x01, 0x3A) => "Counted Buffer",
        (0x01, 0x3B) => "Byte Count",
        (0x01, 0x3C) => "Motion Wakeup",
        (0x01, 0x3D) => "Start",
        (0x01, 0x3E) => "Select",
        (0x01, 0x40) => "Vx",
        (0x01, 0x41) => "Vy",
        (0x01, 0x42) => "Vz",
        (0x01, 0x43) => "Vbrx",
        (0x01, 0x44) => "Vbry",
        (0x01, 0x45) => "Vbrz",
        (0x01, 0x46) => "Vno",
        (0x01, 0x80) => "System Control",
        (0x01, 0x81) => "System Power Down",
        (0x01, 0x82) => "System Sleep",
        (0x01, 0x83) => "System Wake Up",
        (0x01, 0x84) => "System Context Menu",
        (0x01, 0x85) => "System Main Menu",
        (0x01, 0x86) => "System App Menu",
        (0x01, 0x87) => "System Menu Help",
        (0x01, 0x88) => "System Menu Exit",
        (0x01, 0x89) => "System Menu Select",
        (0x01, 0x8A) => "System Menu Right",
        (0x01, 0x8B) => "System Menu Left",
        (0x01, 0x8C) => "System Menu Up",
        (0x01, 0x8D) => "System Menu Down",
        _ => "",
    };
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

// ── Text renderer ──────────────────────────────────────────────────────────────

fn render_nodes(nodes: &[HidNode], base: usize, depth: usize, out: &mut String) {
    let item_indent = base + depth * 6;
    // Collection / End Collection lines sit 4 right of the enclosing depth's items,
    // which is 2 left of their own children (item_indent + 6 - 2 = item_indent + 4).
    let coll_indent = item_indent + 4;

    for node in nodes {
        match node {
            HidNode::Collection { kind, children } => {
                push_line(out, coll_indent, &format!("Collection ({})    ", kind_label(kind)));
                render_nodes(children, base, depth + 1, out);
                push_line(out, coll_indent, "End Collection     ");
            }
            HidNode::UsagePage { name, .. } => {
                push_line(out, item_indent, &format!("Usage Page    ({}) ", name));
            }
            HidNode::Usage { usage_id, name } => match name {
                Some(n) => push_line(out, item_indent, &format!("Usage ({})    ", n)),
                None => push_line(
                    out,
                    item_indent,
                    &format!("Usage {} (0x{:x})    ", usage_id, usage_id),
                ),
            },
            HidNode::ReportId { id } => {
                push_line(out, item_indent, &dot_item("ReportID", &format!("({})", id), false));
            }
            HidNode::LogicalMinimum { value } => {
                push_line(out, item_indent, &dot_item("Logical Minimum", &format!("({})", value), false));
            }
            HidNode::LogicalMaximum { value } => {
                push_line(out, item_indent, &dot_item("Logical Maximum", &format!("({})", value), false));
            }
            HidNode::PhysicalMinimum { value } => {
                push_line(out, item_indent, &dot_item("Physical Minimum", &format!("({})", value), false));
            }
            HidNode::PhysicalMaximum { value } => {
                push_line(out, item_indent, &dot_item("Physical Maximum", &format!("({})", value), false));
            }
            HidNode::UnitExponent { value } => {
                push_line(out, item_indent, &dot_item("Unit Exponent", &format!("({})", value), false));
            }
            HidNode::Unit { value } => {
                push_line(out, item_indent, &dot_item("Unit", &format!("({})", value), false));
            }
            HidNode::ReportSize { value } => {
                push_line(out, item_indent, &dot_item("Report Size", &format!("({})", value), false));
            }
            HidNode::ReportCount { value } => {
                push_line(out, item_indent, &dot_item("Report Count", &format!("({})", value), false));
            }
            HidNode::UsageMinimum { value } => {
                push_line(out, item_indent, &dot_item("Usage Minimum", &format!("({})", value), false));
            }
            HidNode::UsageMaximum { value } => {
                push_line(out, item_indent, &dot_item("Usage Maximum", &format!("({})", value), false));
            }
            HidNode::Input { flags } => {
                push_line(out, item_indent, &dot_item("Input", &format!("({})", input_flags(flags)), true));
            }
            HidNode::Output { flags } => {
                push_line(out, item_indent, &dot_item("Output", &format!("({})", output_flags(flags)), true));
            }
            HidNode::Feature { flags } => {
                push_line(out, item_indent, &dot_item("Feature", &format!("({})", output_flags(flags)), true));
            }
        }
    }
}

fn push_line(out: &mut String, indent: usize, content: &str) {
    for _ in 0..indent {
        out.push(' ');
    }
    out.push_str(content);
    out.push('\n');
}

/// Build a dot-padded label line. Label is padded with dots to 24 chars.
/// `is_io`: true for Input/Output/Feature (3-space gap, 1 trailing); false otherwise (4-space, 2 trailing).
fn dot_item(label: &str, value: &str, is_io: bool) -> String {
    const LABEL_WIDTH: usize = 24;
    let dots = LABEL_WIDTH.saturating_sub(label.len());
    let dot_str = ".".repeat(dots);
    if is_io {
        format!("{}{}   {} ", label, dot_str, value)
    } else {
        format!("{}{}    {}  ", label, dot_str, value)
    }
}

fn kind_label(kind: &CollectionKind) -> &str {
    match kind {
        CollectionKind::Physical => "Physical",
        CollectionKind::Application => "Application",
        CollectionKind::Logical => "Logical",
        CollectionKind::Report => "Report",
        CollectionKind::NamedArray => "Named Array",
        CollectionKind::UsageSwitch => "Usage Switch",
        CollectionKind::UsageModifier => "Usage Modifier",
        CollectionKind::Vendor { .. } => "Vendor-defined",
    }
}

/// Input flags: show all 8 (skipping volatile bit 7) when Variable; only 3 when Array.
fn input_flags(f: &HidIoFlags) -> String {
    if f.variable {
        join_flags(&[
            if f.constant { "Constant" } else { "Data" },
            if f.variable { "Variable" } else { "Array" },
            if f.relative { "Relative" } else { "Absolute" },
            if f.wrap { "Wrap" } else { "No Wrap" },
            if f.non_linear { "Non Linear" } else { "Linear" },
            if f.no_preferred { "No Preferred" } else { "Preferred State" },
            if f.null_state { "Null State" } else { "No Null Position" },
            // bit 7 (Volatile) not applicable to Input — skip
            if f.buffered_bytes { "Buffered Bytes" } else { "Bitfield" },
        ])
    } else {
        // Array input: HID spec says flags 3-8 are not applicable.
        join_flags(&[
            if f.constant { "Constant" } else { "Data" },
            "Array",
            if f.relative { "Relative" } else { "Absolute" },
        ])
    }
}

/// Output/Feature flags: always show all 9 including Volatile (bit 7).
fn output_flags(f: &HidIoFlags) -> String {
    join_flags(&[
        if f.constant { "Constant" } else { "Data" },
        if f.variable { "Variable" } else { "Array" },
        if f.relative { "Relative" } else { "Absolute" },
        if f.wrap { "Wrap" } else { "No Wrap" },
        if f.non_linear { "Non Linear" } else { "Linear" },
        if f.no_preferred { "No Preferred" } else { "Preferred State" },
        if f.null_state { "Null State" } else { "No Null Position" },
        if f.volatile { "Volatile" } else { "Nonvolatile" },
        if f.buffered_bytes { "Buffered Bytes" } else { "Bitfield" },
    ])
}

fn join_flags(flags: &[&str]) -> String {
    flags.join(", ")
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden test: 156-byte HID descriptor from fixture lines 188–197.
    /// Rendered output must match fixture lines 199–276 (items only, no header).
    #[test]
    fn golden_pico2_hid() {
        #[rustfmt::skip]
        let bytes: &[u8] = &[
            0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0x85, 0x01,  0x05, 0x07, 0x19, 0xE0, 0x29, 0xE7, 0x15, 0x00,
            0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02,  0x95, 0x01, 0x75, 0x08, 0x81, 0x01, 0x95, 0x05,
            0x75, 0x01, 0x05, 0x08, 0x19, 0x01, 0x29, 0x05,  0x91, 0x02, 0x95, 0x01, 0x75, 0x03, 0x91, 0x01,
            0x95, 0x06, 0x75, 0x08, 0x15, 0x00, 0x26, 0xFF,  0x00, 0x05, 0x07, 0x19, 0x00, 0x2A, 0xFF, 0x00,
            0x81, 0x00, 0xC0, 0x05, 0x01, 0x09, 0x02, 0xA1,  0x01, 0x09, 0x01, 0xA1, 0x00, 0x85, 0x02, 0x05,
            0x09, 0x19, 0x01, 0x29, 0x05, 0x15, 0x00, 0x25,  0x01, 0x95, 0x05, 0x75, 0x01, 0x81, 0x02, 0x95,
            0x01, 0x75, 0x03, 0x81, 0x01, 0x05, 0x01, 0x09,  0x30, 0x09, 0x31, 0x15, 0x81, 0x25, 0x7F, 0x75,
            0x08, 0x95, 0x02, 0x81, 0x06, 0x09, 0x38, 0x15,  0x81, 0x25, 0x7F, 0x75, 0x08, 0x95, 0x01, 0x81,
            0x06, 0xC0, 0xC0, 0x05, 0x0C, 0x09, 0x01, 0xA1,  0x01, 0x85, 0x03, 0x75, 0x10, 0x95, 0x01, 0x15,
            0x01, 0x26, 0x8C, 0x02, 0x19, 0x01, 0x2A, 0x8C,  0x02, 0x81, 0x00, 0xC0,
        ];

        let nodes = parse(bytes).expect("parse failed");
        let rendered = render_text(&nodes, 26);

        // Expected output extracted from the USB Prober reference fixture (lines 199–276).
        // See tests/golden_pico2_parsed.txt — owned by this crate.
        let expected = include_str!("../tests/golden_pico2_parsed.txt");

        if rendered != expected {
            // Show a diff-friendly output
            let r: Vec<&str> = rendered.lines().collect();
            let e: Vec<&str> = expected.lines().collect();
            for (i, (got, exp)) in r.iter().zip(e.iter()).enumerate() {
                if got != exp {
                    eprintln!("line {}: GOT  {:?}", i + 1, got);
                    eprintln!("line {}: WANT {:?}", i + 1, exp);
                }
            }
            if r.len() != e.len() {
                eprintln!("line count: got {} want {}", r.len(), e.len());
            }
            panic!("golden test failed — see diffs above");
        }
    }
}
