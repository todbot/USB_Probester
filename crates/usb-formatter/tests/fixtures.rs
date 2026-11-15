use usb_types::UsbDevice;

/// Golden tests: render a JSON capture and compare to a crate-owned golden file.
/// Regenerate after intentional formatter changes with:
/// `UPDATE_GOLDEN=1 cargo test -p usb-formatter --test fixtures`
fn check_golden(json: &str, golden_name: &str) {
    let devices: Vec<UsbDevice> = serde_json::from_str(json).expect("bad fixture json");
    let rendered = usb_formatter::format_devices(&devices);

    let golden = format!("{}/tests/{}", env!("CARGO_MANIFEST_DIR"), golden_name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&golden, &rendered).expect("write golden failed");
        return;
    }
    let expected = std::fs::read_to_string(&golden).expect("read golden failed");

    if rendered != expected {
        let r: Vec<&str> = rendered.lines().collect();
        let e: Vec<&str> = expected.lines().collect();
        if let Some(i) = r.iter().zip(e.iter()).position(|(got, exp)| got != exp) {
            eprintln!("line {}: GOT  {:?}", i + 1, r[i]);
            eprintln!("line {}: WANT {:?}", i + 1, e[i]);
        }
        if r.len() != e.len() {
            eprintln!("line count: got {} want {}", r.len(), e.len());
        }
        panic!("{golden_name} failed, see diffs above");
    }
}

#[test]
fn golden_mac2() {
    check_golden(
        include_str!("../../../tests/fixtures/mac2-usb-probester-out.json"),
        "golden_mac2.txt",
    );
}

/// Hub + blink(1) mk3 (HID on iface 0) + CircuitPython Pico (HID on iface 3).
#[test]
fn golden_mac3() {
    check_golden(
        include_str!("../../../tests/fixtures/mac3-usb-probester-out.json"),
        "golden_mac3.txt",
    );
}
