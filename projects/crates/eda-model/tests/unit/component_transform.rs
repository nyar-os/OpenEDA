use super::*;

fn open_design() -> u64 {
    create(CreateRequest::default()).expect("create")["handle"].as_u64().expect("handle")
}

fn place(handle: u64, reference: &str, value: &str, x: f64, y: f64, pins: &[&str]) {
    add_component(
        handle,
        AddComponentRequest {
            sheet: None,
            reference: reference.to_string(),
            value: value.to_string(),
            x,
            y,
            rotation: 0.0,
            pins: pins.iter().map(|pin| (*pin).to_string()).collect(),
        },
    )
    .expect("add component");
}

fn rotate(handle: u64, reference: &str, angle: f64) -> Result<Value, ServiceError> {
    rotate_component(handle, RotateComponentRequest { sheet: None, reference: reference.to_string(), angle })
}

fn mirror(handle: u64, reference: &str, axis: &str) -> Result<Value, ServiceError> {
    mirror_component(handle, MirrorComponentRequest { sheet: None, reference: reference.to_string(), axis: axis.to_string() })
}

fn transform_component(reference: &str, x: f64, y: f64, rotation: f64, mirror: Option<&str>, pins: &[&str]) -> Component {
    Component {
        id: format!("component-{reference}"),
        reference: reference.to_string(),
        value: "10k".to_string(),
        footprint: String::new(),
        x,
        y,
        rotation,
        mirror: mirror.map(str::to_string),
        pins: pins.iter().map(|pin| (*pin).to_string()).collect(),
        pin_types: BTreeMap::new(),
        properties: BTreeMap::new(),
    }
}

#[test]
fn rotate_accumulates_without_wrapping_and_undoes() {
    let handle = open_design();
    place(handle, "R1", "10k", 0.0, 0.0, &["1", "2"]);

    let first = rotate(handle, "R1", 90.0).expect("rotate 90");
    assert_eq!(first["component"]["rotation"], 90.0);
    assert_eq!(first["changed"], true);
    let second = rotate(handle, "R1", 360.0).expect("rotate 360");
    assert_eq!(second["component"]["rotation"], 450.0, "the sum is stored exactly, without wrapping");
    assert_eq!(second["revision"].as_u64().expect("revision"), first["revision"].as_u64().expect("revision") + 1);

    undo(handle).expect("undo");
    let listed = list_components(handle, None).expect("list");
    assert_eq!(listed["components"][0]["rotation"], 90.0, "undo restores the previous turn");
    close(handle).expect("close");
}

#[test]
fn a_zero_angle_reports_no_change_and_adds_no_revision() {
    let handle = open_design();
    place(handle, "R1", "10k", 0.0, 0.0, &["1", "2"]);
    let before = list_components(handle, None).expect("list")["revision"].as_u64().expect("revision");

    let report = rotate(handle, "R1", 0.0).expect("zero rotate");
    assert_eq!(report["changed"], false);
    assert_eq!(report["component"]["rotation"], 0.0);
    assert_eq!(report["revision"].as_u64().expect("revision"), before, "a no-op keeps the revision");

    rotate(handle, "R1", 45.0).expect("rotate 45");
    undo(handle).expect("undo");
    let listed = list_components(handle, None).expect("list");
    assert_eq!(listed["components"][0]["rotation"], 0.0, "the no-op recorded no history, so undo returns to the placement");
    close(handle).expect("close");
}

#[test]
fn rotate_and_mirror_reject_bad_arguments_and_unknown_references() {
    let handle = open_design();
    place(handle, "R1", "10k", 0.0, 0.0, &["1", "2"]);
    let before = list_components(handle, None).expect("list")["revision"].as_u64().expect("revision");

    assert_eq!(rotate(handle, "R9", 90.0).expect_err("unknown reference").code, "object/not-found");
    assert_eq!(rotate(handle, "R1", f64::NAN).expect_err("non-finite angle").code, "session/argument-invalid");
    assert_eq!(mirror(handle, "R9", "x").expect_err("unknown reference").code, "object/not-found");
    assert_eq!(mirror(handle, "R1", "z").expect_err("unknown axis").code, "session/argument-invalid");
    assert_eq!(list_components(handle, None).expect("list")["revision"].as_u64().expect("revision"), before);
    close(handle).expect("close");
}

#[test]
fn mirror_toggles_the_axis_and_the_other_axis_becomes_a_half_turn() {
    let handle = open_design();
    place(handle, "R1", "10k", 0.0, 0.0, &["1", "2"]);

    let first = mirror(handle, "R1", "x").expect("mirror x");
    assert_eq!(first["mirror"], "x");
    assert_eq!(first["component"]["mirror"], "x");
    assert_eq!(first["component"]["rotation"], 0.0);
    assert_eq!(first["changed"], true);

    let cleared = mirror(handle, "R1", "x").expect("mirror x again");
    assert_eq!(cleared["mirror"], Value::Null, "requesting the current axis clears the mirror");
    assert_eq!(cleared["component"]["rotation"], 0.0);

    mirror(handle, "R1", "y").expect("mirror y");
    let swapped = mirror(handle, "R1", "x").expect("switch to x");
    assert_eq!(swapped["mirror"], Value::Null);
    assert_eq!(swapped["component"]["rotation"], 180.0, "the second axis is the rotation equivalent");
    close(handle).expect("close");
}

/// True when two design points agree within a rounding tolerance.
fn points_close(left: [f64; 2], right: [f64; 2]) -> bool {
    (left[0] - right[0]).abs() < 1e-9 && (left[1] - right[1]).abs() < 1e-9
}

#[test]
fn mirrored_pins_and_artwork_flip_in_the_local_frame() {
    let unmirrored = component_pins(&transform_component("R1", 0.0, 0.0, 0.0, None, &["1", "2"]));
    assert!(points_close(unmirrored[0].root, [-COMPONENT_HALF_WIDTH, 0.0]));
    assert!(points_close(unmirrored[0].end, [-(COMPONENT_HALF_WIDTH + PIN_LENGTH), 0.0]));
    assert!(unmirrored[0].left);

    let flipped = component_pins(&transform_component("R1", 0.0, 0.0, 0.0, Some("x"), &["1", "2"]));
    assert!(points_close(flipped[0].root, [COMPONENT_HALF_WIDTH, 0.0]), "the x mirror swaps the sides");
    assert!(points_close(flipped[0].end, [COMPONENT_HALF_WIDTH + PIN_LENGTH, 0.0]));
    assert!(!flipped[0].left);
    assert!(points_close(flipped[1].root, [-COMPONENT_HALF_WIDTH, 0.0]));
    assert!(flipped[1].left);

    let four = transform_component("U1", 0.0, 0.0, 0.0, Some("y"), &["1", "2", "3", "4"]);
    let vertical = component_pins(&four);
    assert!(
        points_close(vertical[0].root, [-COMPONENT_HALF_WIDTH, PIN_PITCH / 2.0]),
        "the y mirror swaps the rows inside each side"
    );
    assert!(points_close(vertical[1].root, [-COMPONENT_HALF_WIDTH, -PIN_PITCH / 2.0]));
    assert!(vertical[0].left && vertical[1].left, "the y mirror keeps the sides");

    let resistor = symbol_artwork(&transform_component("R1", 0.0, 0.0, 0.0, Some("x"), &["1", "2"]));
    let SymbolPrimitive::Stroke(lead) = &resistor.0[0]
    else {
        panic!("the first resistor primitive is a lead");
    };
    assert!(points_close(lead[0], [COMPONENT_HALF_WIDTH, 0.0]), "the artwork roots follow the mirrored pins");
    let SymbolPrimitive::Stroke(zigzag) = &resistor.0[1]
    else {
        panic!("the second resistor primitive is the zigzag");
    };
    assert!(points_close(zigzag[0], [5.08, 0.0]), "the zigzag starts from the mirrored body edge");
}

#[test]
fn transforms_survive_save_and_reopen() {
    let handle = open_design();
    place(handle, "R1", "10k", 0.0, 0.0, &["1", "2"]);
    place(handle, "C1", "100n", 40.0, 0.0, &["1", "2"]);
    rotate(handle, "R1", 90.0).expect("rotate");
    mirror(handle, "C1", "y").expect("mirror");
    let path = std::env::temp_dir().join(format!("open-eda-transform-{}.oeda", std::process::id()));
    save(handle, &path, true).expect("save");
    close(handle).expect("close");

    let reopened = open(&path).expect("reopen");
    let handle = reopened["handle"].as_u64().expect("handle");
    let listed = list_components(handle, None).expect("list");
    let components = listed["components"].as_array().expect("components");
    assert_eq!(components[0]["rotation"], 90.0);
    assert_eq!(components[0]["mirror"], Value::Null, "an unmirrored component omits the field");
    assert_eq!(components[1]["mirror"], "y");
    close(handle).expect("close");
    let _ = fs::remove_file(&path);
}

#[test]
fn kicad_exchange_round_trips_the_mirror() {
    let handle = open_design();
    place(handle, "R1", "10k", 0.0, 0.0, &["1", "2"]);
    mirror(handle, "R1", "x").expect("mirror x");
    let path = std::env::temp_dir().join(format!("open-eda-transform-kicad-{}.kicad_sch", std::process::id()));
    export(handle, "kicad_sch", &path).expect("export");
    close(handle).expect("close");

    let text = fs::read_to_string(&path).expect("read export");
    assert!(text.contains("(mirror \"x\")"), "the writer emits the KiCad mirror token");
    let (design, losses) = kicad::import(&text).expect("import");
    assert!(losses.is_empty(), "the mirror is inside the imported subset: {losses:?}");
    assert_eq!(design.sheets[0].components[0].mirror.as_deref(), Some("x"));
    let _ = fs::remove_file(&path);
}

#[test]
fn validation_refuses_a_mirror_outside_the_axis_names() {
    let mut design = Design {
        id: "design-1".to_string(),
        title: None,
        rev: String::new(),
        company: String::new(),
        date: String::new(),
        revision: 1,
        units: "mm".to_string(),
        sheets: vec![Sheet {
            id: "sheet-1".to_string(),
            name: "Root".to_string(),
            components: vec![transform_component("R1", 0.0, 0.0, 0.0, Some("z"), &["1", "2"])],
            nets: Vec::new(),
            wires: Vec::new(),
            junctions: Vec::new(),
            no_connects: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
        }],
        buses: Vec::new(),
        variants: Vec::new(),
        rules: DesignRules::default(),
        hierarchy: Vec::new(),
        next_component: 2,
        next_net: 1,
        next_wire: 1,
    };
    assert_eq!(validate_design(&design).expect_err("unknown axis").code, "format/invalid");
    design.sheets[0].components[0].mirror = Some("x".to_string());
    validate_design(&design).expect("x is a valid axis");
}

#[test]
fn the_matrix_reports_component_transforms_as_supported() {
    let matrix = capabilities();
    assert_eq!(matrix["edits"]["rotate"], "supported");
    assert_eq!(matrix["edits"]["mirror"], "supported");
}
