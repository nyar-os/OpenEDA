
use super::*;

#[test]
fn coordinates_keep_four_decimals_and_drop_trailing_zeros() {
    assert_eq!(num(40.0), "40");
    assert_eq!(num(-0.0), "0");
    assert_eq!(num(12.34567), "12.3457");
    assert_eq!(num(1.5), "1.5");
    assert_eq!(num(0.0000001), "0");
    assert_eq!(num(-2.25), "-2.25");
    assert_eq!(num(f64::NAN), "0");
}

#[test]
fn uuids_and_library_ids_are_deterministic() {
    assert_eq!(uuid_for("design-1:component:component-1"), uuid_for("design-1:component:component-1"));
    assert_ne!(uuid_for("design-1:component:component-1"), uuid_for("design-1:component:component-2"));
    let uuid = uuid_for("seed");
    assert!(uuid.starts_with("00000000-0000-0000-0000-"));
    assert_eq!(uuid.len(), 36);

    let numeric = Component {
        id: "component-1".to_string(),
        reference: "R1".to_string(),
        value: "10k".to_string(),
        footprint: String::new(),
        x: 0.0,
        y: 0.0,
        rotation: 0.0,
        pins: vec!["1".to_string(), "2".to_string()],
        pin_types: Default::default(),
        properties: Default::default(),
    };
    assert_eq!(lib_id_for(&numeric), "open-eda:2pin");
    let named = Component { pins: vec!["A".to_string(), "K".to_string()], ..numeric.clone() };
    assert!(lib_id_for(&named).starts_with("open-eda:2pin_"));
    assert_eq!(lib_id_for(&named), lib_id_for(&named.clone()));
    let mut typed = numeric.clone();
    typed.pin_types.insert("1".to_string(), PinType::Output);
    assert_ne!(lib_id_for(&numeric), lib_id_for(&typed));
    assert!(lib_id_for(&typed).starts_with("open-eda:2pin_"));
    assert_eq!(lib_id_for(&typed), lib_id_for(&typed.clone()));
}

#[test]
fn strings_escape_quotes_and_backslashes() {
    assert_eq!(quote("R1"), "\"R1\"");
    assert_eq!(quote("10k (1%)"), "\"10k (1%)\"");
    assert_eq!(quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
    let parsed = parse(&format!("(label {})", quote("a\"b\\c"))).expect("parse");
    assert_eq!(node_text(&parsed[0].children()[0]), Some("a\"b\\c"));
}
