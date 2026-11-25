use super::*;
use crate::{Component, Design, DesignRules, Net, Node, Sheet};
use std::collections::BTreeMap;

/// Builds one component with the given reference, value and pins.
fn component(reference: &str, value: &str, x: f64, y: f64, pins: &[&str]) -> Component {
    Component {
        id: format!("component-{reference}"),
        reference: reference.to_string(),
        value: value.to_string(),
        footprint: String::new(),
        x,
        y,
        rotation: 0.0,
        mirror: None,
        pins: pins.iter().map(|pin| pin.to_string()).collect(),
        pin_types: BTreeMap::new(),
        properties: BTreeMap::new(),
    }
}

/// Counts non-overlapping occurrences of `needle` in `haystack`.
fn occurrences(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// One design with two sheets, shared nets, a duplicate-pin net, a single-member
/// net and a no-connect flag, exercising every drawing rule.
fn sample_design() -> Design {
    Design {
        id: "drawings-design".to_string(),
        title: Some("Drawings".to_string()),
        rev: "A".to_string(),
        company: String::new(),
        date: String::new(),
        revision: 1,
        units: "mm".to_string(),
        sheets: vec![
            Sheet {
                id: "sheet-1".to_string(),
                name: "Power".to_string(),
                components: vec![
                    component("U1", "MCU", 60.0, 40.0, &["VCC", "GND", "IO1", "IO2"]),
                    component("R1", "10k", 95.0, 25.0, &["1", "2"]),
                    component("R2", "330", 95.0, 55.0, &["1", "2"]),
                    component("C1", "100n", 20.0, 38.0, &["1", "2"]),
                    component("D1", "LED", 130.0, 55.0, &["A", "K"]),
                    component("J1", "CONN", 145.0, 25.0, &["1", "2"]),
                    component("R3", "10k", 30.0, 80.0, &["1", "2"]),
                ],
                nets: vec![
                    Net {
                        id: "net-1".to_string(),
                        name: "VCC".to_string(),
                        nodes: vec![
                            Node { reference: "U1".to_string(), pin: "VCC".to_string() },
                            Node { reference: "R1".to_string(), pin: "2".to_string() },
                            Node { reference: "R1".to_string(), pin: "1".to_string() },
                            Node { reference: "C1".to_string(), pin: "2".to_string() },
                        ],
                    },
                    Net {
                        id: "net-2".to_string(),
                        name: "GND".to_string(),
                        nodes: vec![
                            Node { reference: "C1".to_string(), pin: "1".to_string() },
                            Node { reference: "U1".to_string(), pin: "GND".to_string() },
                            Node { reference: "D1".to_string(), pin: "K".to_string() },
                        ],
                    },
                    Net {
                        id: "net-3".to_string(),
                        name: "IO1".to_string(),
                        nodes: vec![
                            Node { reference: "U1".to_string(), pin: "IO1".to_string() },
                            Node { reference: "R2".to_string(), pin: "1".to_string() },
                        ],
                    },
                    Net {
                        id: "net-4".to_string(),
                        name: "LED".to_string(),
                        nodes: vec![
                            Node { reference: "R2".to_string(), pin: "2".to_string() },
                            Node { reference: "D1".to_string(), pin: "A".to_string() },
                        ],
                    },
                    Net {
                        id: "net-5".to_string(),
                        name: "SPARE".to_string(),
                        nodes: vec![Node { reference: "J1".to_string(), pin: "1".to_string() }],
                    },
                ],
                wires: Vec::new(),
                junctions: Vec::new(),
                no_connects: vec![crate::NoConnect {
                    x: 160.0,
                    y: 25.0,
                    node: Some(Node { reference: "J1".to_string(), pin: "2".to_string() }),
                }],
                texts: Vec::new(),
                labels: Vec::new(),
            },
            Sheet {
                id: "sheet-2".to_string(),
                name: "Output".to_string(),
                components: vec![component("Q1", "BJT", 40.0, 30.0, &["B", "C", "E"])],
                nets: vec![Net {
                    id: "net-6".to_string(),
                    name: "SIG".to_string(),
                    nodes: vec![
                        Node { reference: "Q1".to_string(), pin: "B".to_string() },
                        Node { reference: "Q1".to_string(), pin: "C".to_string() },
                    ],
                }],
                wires: vec![crate::Wire {
                    id: "wire-1".to_string(),
                    net: Some("SIG".to_string()),
                    from: [10.0, 10.0],
                    to: [90.0, 10.0],
                }],
                junctions: Vec::new(),
                no_connects: Vec::new(),
                texts: Vec::new(),
                labels: Vec::new(),
            },
        ],
        buses: Vec::new(),
        variants: Vec::new(),
        hierarchy: Vec::new(),
        rules: DesignRules::default(),
        next_component: 9,
        next_net: 7,
        next_wire: 2,
    }
}

#[test]
fn block_diagram_layers_nodes_and_chains_each_multi_member_net() {
    let design = sample_design();
    let drawing = render_block_diagram(&design);
    let svg = &drawing.svg;

    assert!(svg.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"), "the drawing is a standalone SVG document");
    assert!(svg.contains(r#"data-block-diagram="true""#), "the diagram carries its own group marker");
    assert!(svg.contains(r#"data-title-block="true""#), "the shared title block is painted");

    for reference in ["U1", "R1", "R2", "C1", "D1", "J1", "R3", "Q1"] {
        assert!(svg.contains(&format!(r#"data-block="{reference}""#)), "every component becomes a node: {reference}");
    }
    assert_eq!(drawing.nodes, 8, "one node per component");

    // VCC deduplicates R1's two pins to one member: three members, two segments.
    assert_eq!(occurrences(svg, r#"data-net="VCC""#), 2, "a net chain has members-1 segments");
    assert_eq!(occurrences(svg, r#"data-net-name="VCC""#), 1, "the net name is printed once per chain");
    assert_eq!(occurrences(svg, r#"data-net="GND""#), 2, "GND chains three members");
    assert_eq!(occurrences(svg, r#"data-net="IO1""#), 1, "a two-member net paints one segment");
    assert!(!svg.contains(r#"data-net="SPARE""#), "a single-member net is not routed");
    assert_eq!(drawing.nets, 4, "VCC, GND, IO1 and LED are multi-member; SIG dedupes to one member");
    assert_eq!(drawing.edges, 6, "2 + 2 + 1 + 1 segments");

    // Breadth-first layering starts from the first reference of each island,
    // so C1 roots the main island at the left margin and its VCC/GND partner
    // MCU lands one column right (node width 30 + gap 26).
    let c1_x = svg.find(r#"data-block="C1""#).expect("C1 node");
    let u1_x = svg.find(r#"data-block="U1""#).expect("U1 node");
    let c1_rect = &svg[c1_x..c1_x + 220];
    let u1_rect = &svg[u1_x..u1_x + 220];
    assert!(c1_rect.contains(r#"x="10.00""#), "the BFS root sits at the left margin, got: {c1_rect}");
    assert!(u1_rect.contains(r#"x="66.00""#), "a net partner is layered one column right, got: {u1_rect}");

    assert_eq!(svg, &render_block_diagram(&design).svg, "the layout is deterministic");
}

#[test]
fn block_diagram_of_an_empty_design_still_paints_a_document() {
    let mut design = sample_design();
    for sheet in &mut design.sheets {
        sheet.components.clear();
        sheet.nets.clear();
    }
    let drawing = render_block_diagram(&design);
    assert!(drawing.svg.ends_with("</svg>\n"), "an empty design still closes the document");
    assert_eq!(drawing.nodes, 0);
    assert_eq!(drawing.edges, 0);
    assert!(drawing.svg.contains(r#"data-title-block="true""#), "the title block survives");
}

#[test]
fn sheet_overview_grids_every_sheet_under_one_title_block() {
    let design = sample_design();
    let drawing = render_sheet_overview(&design);
    let svg = &drawing.svg;

    assert!(svg.contains(r#"data-sheet-overview="true""#), "the overview carries its group marker");
    assert!(svg.contains(r#"data-sheet="sheet-1""#) && svg.contains(r#"data-sheet="sheet-2""#), "both sheets are placed");
    assert_eq!(occurrences(svg, r#"data-title-block="true""#), 1, "exactly one shared title block");
    assert_eq!(drawing.sheets, 2);
    assert_eq!((drawing.columns, drawing.rows), (2, 1), "two sheets fill one row of two cells");

    // Every sheet is painted through the shared painter inside its own
    // transform, so the sheet geometry survives without a second copy.
    assert!(svg.contains(r#"transform="translate("#), "cells scale their sheet contents");
    assert!(svg.contains(">Power<") && svg.contains(">Output<"), "each cell names its sheet");
    assert!(svg.contains(r#"stroke-dasharray="2.4 1.6""#), "cells carry separator frames");
    assert_eq!(svg, &render_sheet_overview(&design).svg, "the grid is deterministic");
}

#[test]
fn pinout_chart_maps_pins_to_nets_and_marks_no_connects() {
    let design = sample_design();
    let drawing = render_pinout_chart(&design);
    let svg = &drawing.svg;

    assert!(svg.contains(r#"data-pinout-chart="true""#), "the chart carries its group marker");
    assert!(svg.contains(r#"data-ref="U1""#), "the MCU block is present");
    assert!(svg.contains(r#"data-pin="VCC""#), "U1 lists its VCC pin row");
    assert!(svg.contains(">VCC, VCC<") || svg.contains(r#">VCC</text>"#), "the pin row names its net");

    // J1.2 carries the no-connect flag, so its row shows the red NC marker.
    let j1 = svg.find(r#"data-ref="J1""#).expect("J1 block");
    let j1_block = &svg[j1..];
    assert!(j1_block.contains(r#"data-no-connect="true""#), "a flagged pin is marked NC");
    assert!(j1_block.contains(">NC</text>"), "the NC marker is printed as text");

    // Q1.E has no net node, so its row prints the em dash placeholder.
    let q1 = svg.find(r#"data-ref="Q1""#).expect("Q1 block");
    assert!(svg[q1..].contains("—"), "an unconnected pin prints a dash");

    assert_eq!(drawing.components, 8, "one block per component");
    assert_eq!(drawing.rows, 19, "4+2+2+2+2+2+2+3 pin rows");
    assert_eq!(svg, &render_pinout_chart(&design).svg, "the chart is deterministic");
}

#[test]
fn symbol_library_dedupes_symbols_by_prefix_and_pins() {
    let design = sample_design();
    let drawing = render_symbol_library(&design);
    let svg = &drawing.svg;

    assert!(svg.contains(r#"data-symbol-library="true""#), "the sheet carries its group marker");
    // R1, R2 and R3 share the `R` prefix and pins 1,2, so one cell covers them.
    assert!(svg.contains(r#"data-symbol="R|1|2""#), "the resistor symbol is keyed by prefix and pins");
    assert!(svg.contains(r#"data-symbol="U|VCC|GND|IO1|IO2""#), "the MCU symbol is keyed by its pin list");
    assert_eq!(drawing.symbols, 6, "R, C, U, D, J and Q symbols");
    assert_eq!(drawing.components, 8, "deduplicated from eight placed parts");

    // The first reference in natural order represents each symbol.
    assert!(svg.contains(r#"data-ref="R1""#), "R1 represents the resistor symbol");
    assert!(!svg.contains(r#"data-ref="R2""#), "R2 shares the R1 cell and is not painted again");
    assert!(!svg.contains(r#"data-ref="R3""#), "R3 shares the R1 cell and is not painted again");
    assert!(svg.contains(r#"transform="rotate(0.00 "#), "library symbols are drawn upright");
    assert_eq!(svg, &render_symbol_library(&design).svg, "the library is deterministic");
}
