use crate::{
    Component, Design, DesignRules, Net, Node, Sheet,
    drawings::{
        render_block_diagram, render_bom_drawing, render_net_topology, render_net_xref, render_pinout_chart,
        render_power_distribution, render_sheet_overview, render_symbol_library,
    },
};
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

/// Extracts every `(name, x, y)` of a `data-net-name` text element.
fn net_name_labels(svg: &str) -> Vec<(String, f64, f64)> {
    let mut labels = Vec::new();
    let mut rest = svg;
    while let Some(at) = rest.find("data-net-name=\"") {
        let after = &rest[at + "data-net-name=\"".len()..];
        let name = after[..after.find('"').expect("closing quote")].to_string();
        let tag_start = rest[..at].rfind("<text ").expect("text element start");
        let attributes = &rest[tag_start..at];
        let number_after = |key: &str| -> f64 {
            let at = attributes.find(key).expect("attribute");
            let value = &attributes[at + key.len()..];
            value[..value.find('"').expect("closing quote")].parse().expect("number")
        };
        labels.push((name, number_after("x=\""), number_after(" y=\"")));
        rest = &after[after.find('>').expect("element end") + 1..];
    }
    labels
}

/// A design where two nets wire the same component pair and two more wire the
/// other pair, so every net name would land on the identical label point.
fn star_design() -> Design {
    Design {
        id: "star-design".to_string(),
        title: None,
        rev: String::new(),
        company: String::new(),
        date: String::new(),
        revision: 1,
        units: "mm".to_string(),
        sheets: vec![Sheet {
            id: "sheet-1".to_string(),
            name: "Star".to_string(),
            components: vec![
                component("R1", "1", 10.0, 10.0, &["1", "2", "3", "4"]),
                component("U1", "2", 60.0, 10.0, &["1", "2", "3", "4"]),
                component("R2", "3", 120.0, 10.0, &["1", "2", "3", "4"]),
            ],
            nets: vec![
                Net {
                    id: "net-a".to_string(),
                    name: "AAAA".to_string(),
                    nodes: vec![
                        Node { reference: "R1".to_string(), pin: "1".to_string() },
                        Node { reference: "U1".to_string(), pin: "1".to_string() },
                    ],
                },
                Net {
                    id: "net-b".to_string(),
                    name: "BBBB".to_string(),
                    nodes: vec![
                        Node { reference: "R1".to_string(), pin: "2".to_string() },
                        Node { reference: "U1".to_string(), pin: "2".to_string() },
                    ],
                },
                Net {
                    id: "net-c".to_string(),
                    name: "CCCC".to_string(),
                    nodes: vec![
                        Node { reference: "U1".to_string(), pin: "3".to_string() },
                        Node { reference: "R2".to_string(), pin: "1".to_string() },
                    ],
                },
                Net {
                    id: "net-d".to_string(),
                    name: "DDDD".to_string(),
                    nodes: vec![
                        Node { reference: "U1".to_string(), pin: "4".to_string() },
                        Node { reference: "R2".to_string(), pin: "2".to_string() },
                    ],
                },
            ],
            wires: Vec::new(),
            junctions: Vec::new(),
            no_connects: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
        }],
        buses: Vec::new(),
        variants: Vec::new(),
        hierarchy: Vec::new(),
        rules: DesignRules::default(),
        next_component: 4,
        next_net: 5,
        next_wire: 1,
    }
}

#[test]
fn block_diagram_pushes_colliding_net_names_apart() {
    let design = star_design();
    let drawing = render_block_diagram(&design);
    let labels = net_name_labels(&drawing.svg);
    assert_eq!(labels.len(), 4, "every multi-member chain names itself once");

    // AAAA/BBBB share the R1-U1 chain point and CCCC/DDDD share U1-R2's, so a
    // naive layout would paint them on top of each other.
    let box_of = |label: &(String, f64, f64)| {
        let half = label.0.chars().count() as f64 * 2.4 * 0.3;
        [label.1 - half, label.2 - 2.4, label.1 + half, label.2]
    };
    for (index, left) in labels.iter().enumerate() {
        for right in &labels[index + 1..] {
            let (a, b) = (box_of(left), box_of(right));
            let disjoint = a[2] <= b[0] || b[2] <= a[0] || a[3] <= b[1] || b[3] <= a[1];
            assert!(disjoint, "labels `{}` and `{}` overlap: {:?} vs {:?}", left.0, right.0, a, b);
        }
    }
    let view = &drawing.svg[drawing.svg.find("viewBox=\"").expect("viewBox") + 9..];
    let view = &view[..view.find('"').expect("viewBox end")];
    let numbers = view.split_whitespace().map(str::parse::<f64>).collect::<Result<Vec<_>, _>>().expect("viewBox numbers");
    let (view_x, view_y, view_width, view_height) = (numbers[0], numbers[1], numbers[2], numbers[3]);
    for label in &labels {
        let bounds = box_of(label);
        assert!(
            bounds[0] >= view_x && bounds[2] <= view_x + view_width && bounds[1] >= view_y && bounds[3] <= view_y + view_height,
            "label `{}` leaves the canvas: {:?} outside {:?}",
            label.0,
            bounds,
            (view_x, view_y, view_width, view_height)
        );
    }
}

#[test]
fn multi_sheet_svg_stacks_sheets_into_separate_bands() {
    let design = sample_design();
    let sheets = design.sheets.iter().collect::<Vec<_>>();
    let svg = crate::render_svg_for(&design, &sheets);

    // One labelled band per sheet, each with its own separator frame.
    assert_eq!(occurrences(&svg, "stroke-dasharray=\"2.4 1.6\""), 2, "every sheet gets a band frame");
    assert_eq!(occurrences(&svg, "data-sheet=\""), 2, "each sheet is painted once");

    // The band frames are stacked, never overlapping.
    let mut band_bottom = f64::NEG_INFINITY;
    let mut rest = svg.as_str();
    let needle = "stroke-dasharray=\"2.4 1.6\"";
    while let Some(at) = rest.find(needle) {
        let tag_start = rest[..at].rfind("<rect ").expect("band rect");
        let tag = &rest[tag_start..at];
        let number_after = |key: &str| -> f64 {
            let at = tag.find(key).expect("attribute");
            let value = &tag[at + key.len()..];
            value[..value.find('"').expect("closing quote")].parse().expect("number")
        };
        let top = number_after(" y=\"");
        let height = number_after("height=\"");
        assert!(top >= band_bottom - 0.01, "bands overlap: {top} starts before {band_bottom}");
        band_bottom = top + height;
        rest = &rest[at + needle.len()..];
    }

    // Each sheet group is translated into its own band, so identical design
    // coordinates land in different device rows.
    let translate_of = |sheet_id: &str| -> f64 {
        let at = svg.find(&format!("data-sheet=\"{sheet_id}\" transform=\"translate(")).expect("sheet group");
        let values = &svg[at..];
        let open = values.find("translate(").expect("translate") + "translate(".len();
        let close = values[open..].find(')').expect("translate end");
        values[open..open + close].split_whitespace().nth(1).expect("y").parse().expect("number")
    };
    assert!(translate_of("sheet-2") > translate_of("sheet-1"), "the second sheet must sit below the first");

    // The document viewport covers every band plus the title block.
    let view = &svg[svg.find("viewBox=\"").expect("viewBox") + 9..];
    let view = &view[..view.find('"').expect("viewBox end")];
    let view_height: f64 = view.split_whitespace().nth(3).expect("height").parse().expect("number");
    assert!(band_bottom <= view_height - 10.0 + 0.01, "bands must stay inside the canvas, got {band_bottom} of {view_height}");
    assert!(svg.contains(r#"data-title-block="true""#), "the shared title block survives paging");
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

    // The layout splits tall BFS layers into columns and pad the page, so the
    // root no longer has to sit at x = 10. What matters is that the partner is
    // layered to the right and the whole grid is painted.
    let node_x = |reference: &str| -> f64 {
        let at = svg.find(&format!(r#"data-block="{reference}""#)).expect("node group");
        let rect = at + svg[at..].find("<rect x=\"").expect("node rect") + "<rect x=\"".len();
        let value = &svg[rect..];
        value[..value.find('"').expect("quote")].parse().expect("number")
    };
    assert!(node_x("U1") > node_x("C1"), "a net partner is layered to the right");
    assert!(node_x("C1") >= 0.0, "nodes stay inside the content box");

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

    // Q1.E has no net node, so its row stays blank rather than filling the
    // column with placeholder dashes.
    let q1 = svg.find(r#"data-ref="Q1""#).expect("Q1 block");
    assert!(!svg.contains("—"), "unconnected pins do not print a dash");
    assert!(svg[q1..].contains(r#"data-pin="E""#), "the unconnected pin still has a row");

    assert_eq!(drawing.components, 8, "one block per component");
    assert_eq!(drawing.rows, 19, "4+2+2+2+2+2+2+3 pin rows");
    assert_eq!(svg, &render_pinout_chart(&design).svg, "the chart is deterministic");
}

#[test]
fn oversized_labels_widen_the_measured_column_instead_of_clipping() {
    let mut design = sample_design();
    design.sheets[0].components[0].value = "V".repeat(60);
    let drawing = render_pinout_chart(&design);
    let at = drawing.svg.find(r#"data-ref="U1""#).expect("U1 block");
    let text_at = at + drawing.svg[at..].find("<text").expect("header text");
    let tag_end = drawing.svg[text_at..].find('>').expect("tag end");
    let shown = &drawing.svg[text_at + tag_end + 1..];
    let shown = &shown[..shown.find('<').expect("text end")];
    assert!(shown.contains(&"V".repeat(60)), "the measured header keeps the whole value: {shown}");
    assert!(!shown.ends_with("..."), "no truncation when the column can grow: {shown}");
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

/// Collects the painted box of every `<text>` element, estimating monospace
/// run widths at 0.6 em and honouring the `text-anchor` variants.
fn text_boxes(svg: &str) -> Vec<[f64; 4]> {
    let mut boxes = Vec::new();
    let mut rest = svg;
    while let Some(at) = rest.find("<text ") {
        let tag_end = at + rest[at..].find('>').expect("tag end");
        let tag = &rest[at..tag_end];
        let number_after = |key: &str| -> Option<f64> {
            tag.find(key).map(|a| tag[a + key.len()..].split('"').next().expect("value").parse().expect("number"))
        };
        let x = number_after("x=\"").expect("x");
        let y = number_after(" y=\"").expect("y");
        let size = number_after("font-size=\"").expect("font-size");
        let content = &rest[tag_end + 1..];
        let text = &content[..content.find('<').expect("element end")];
        let width = text.chars().count() as f64 * size * 0.6;
        let left = if tag.contains("text-anchor=\"middle\"") {
            x - width / 2.0
        }
        else if tag.contains("text-anchor=\"end\"") {
            x - width
        }
        else {
            x
        };
        boxes.push([left, y - size * 0.8, left + width, y]);
        rest = &content[text.len() + 1..];
    }
    boxes
}

/// Asserts every text box lies inside the document viewBox.
fn assert_inside_canvas(svg: &str, boxes: &[[f64; 4]]) {
    let view = &svg[svg.find("viewBox=\"").expect("viewBox") + 9..];
    let view = &view[..view.find('"').expect("viewBox end")];
    let numbers = view.split_whitespace().map(str::parse::<f64>).collect::<Result<Vec<_>, _>>().expect("viewBox numbers");
    for box_edges in boxes {
        assert!(
            box_edges[0] >= numbers[0] - 0.01
                && box_edges[2] <= numbers[0] + numbers[2] + 0.01
                && box_edges[1] >= numbers[1] - 0.01
                && box_edges[3] <= numbers[1] + numbers[3] + 0.01,
            "text leaves the canvas: {box_edges:?} outside {numbers:?}"
        );
    }
}

/// The y of the first `<rect>` element at or after `from`.
fn rect_y_in(svg: &str, from: usize) -> f64 {
    let at = from + svg[from..].find("<rect ").expect("rect");
    let end = at + svg[at..].find('>').expect("rect end");
    let tag = &svg[at..end];
    let y = &tag[tag.find(" y=\"").expect("y") + 4..];
    y[..y.find('"').expect("quote")].parse().expect("number")
}

/// Asserts no two text boxes intersect.
fn assert_no_text_overlap(boxes: &[[f64; 4]]) {
    for (index, left) in boxes.iter().enumerate() {
        for right in &boxes[index + 1..] {
            let disjoint = left[2] <= right[0] || right[2] <= left[0] || left[3] <= right[1] || right[3] <= left[1];
            assert!(disjoint, "text boxes overlap: {left:?} vs {right:?}");
        }
    }
}

#[test]
fn net_topology_paints_the_component_net_incidence_graph() {
    let design = sample_design();
    let drawing = render_net_topology(&design);
    let svg = &drawing.svg;

    assert!(svg.contains(r#"data-net-topology="true""#), "the sheet carries its group marker");
    // Six nets carry known members: VCC(3) GND(3) IO1(2) LED(2) SPARE(1) SIG(1).
    assert_eq!(drawing.components, 8, "one box per component");
    assert_eq!(drawing.nets, 6, "one node per net with members");
    assert_eq!(drawing.edges, 12, "one edge per component-net incidence");
    assert_eq!(occurrences(svg, "<polyline"), 12, "edges are routed");
    assert_eq!(svg, &render_net_topology(&design).svg, "the graph is deterministic");

    // Single-member nets stay visible, unlike in the block diagram chain.
    assert!(svg.contains(r#"data-net-node="SPARE""#), "single-member nets are painted");

    let boxes = text_boxes(svg);
    assert_inside_canvas(svg, &boxes);
    assert_no_text_overlap(&boxes);
}

#[test]
fn power_distribution_groups_rails_by_net_convention() {
    let design = sample_design();
    let drawing = render_power_distribution(&design);
    let svg = &drawing.svg;

    assert!(svg.contains(r#"data-power-distribution="true""#), "the sheet carries its group marker");
    assert_eq!(drawing.power_nets, 1, "VCC is the only power net");
    assert_eq!(drawing.ground_nets, 1, "GND is the only ground net");
    assert_eq!(drawing.components, 8, "every component is hung on the map");
    assert_eq!(drawing.connections, 6, "VCC has three members and GND has three");
    assert!(svg.contains(r#"data-rail="power" data-net="VCC""#), "the VCC rail bar is painted");
    assert!(svg.contains(r#"data-rail="ground" data-net="GND""#), "the GND rail bar is painted");
    assert!(!svg.contains(r#"data-net="IO1""#), "signal nets are not rails");

    // The power rail sits above the component field and the ground below.
    let rect_y = |marker: &str| -> f64 {
        let mut rest: &str = svg;
        loop {
            let at = rest.find("<rect ").expect("rect");
            let end = at + rest[at..].find('>').expect("rect end");
            let tag = &rest[at..end];
            if tag.contains(marker) {
                let y = &tag[tag.find(" y=\"").expect("y") + 4..];
                return y[..y.find('"').expect("quote")].parse().expect("number");
            }
            rest = &rest[end..];
        }
    };
    let power_y = rect_y(r#"data-rail="power""#);
    let ground_y = rect_y(r#"data-rail="ground""#);
    let group = svg.find("data-block=").expect("component group");
    let box_y = rect_y_in(svg, group);
    assert!(power_y < box_y, "power rails sit above the components");
    assert!(ground_y > box_y, "ground rails sit below the components");

    let boxes = text_boxes(svg);
    assert_inside_canvas(svg, &boxes);
    assert_no_text_overlap(&boxes);
}

#[test]
fn power_distribution_honours_power_pin_types_and_notes_railless_designs() {
    let mut design = sample_design();
    design.sheets[0].components[0].pin_types.insert("IO1".to_string(), crate::PinType::PowerIn);
    let drawing = render_power_distribution(&design);
    assert!(drawing.svg.contains(r#"data-net="IO1""#), "a PowerIn pin promotes the net to a power rail");
    assert_eq!(drawing.power_nets, 2, "VCC plus the promoted IO1 rail");

    let mut railless = sample_design();
    railless.sheets[0].nets.retain(|net| net.name == "IO1" || net.name == "LED");
    railless.sheets[1].nets.retain(|net| net.name == "SIG");
    let drawing = render_power_distribution(&railless);
    assert_eq!(drawing.power_nets, 0, "no rail names survive the filter");
    assert_eq!(drawing.ground_nets, 0, "no ground names survive the filter");
    assert!(drawing.svg.contains("no power or ground nets detected"), "the map explains its emptiness");
    let boxes = text_boxes(&drawing.svg);
    assert_inside_canvas(&drawing.svg, &boxes);
}

#[test]
fn bom_drawing_paints_the_grouped_table() {
    let design = sample_design();
    let drawing = render_bom_drawing(&design);
    let svg = &drawing.svg;

    assert!(svg.contains(r#"data-bom-drawing="true""#), "the sheet carries its group marker");
    assert_eq!(drawing.rows, 7, "eight parts group into seven value/footprint rows");
    assert_eq!(drawing.components, 8, "the rows cover every part");
    assert_eq!(occurrences(svg, "data-bom-cell="), 35, "five columns per row");
    assert!(svg.contains(">R1, R3<"), "the two 10k parts share one designator cell");
    assert!(svg.contains(r#"data-title-block="true""#), "the table carries the shared title block");
    assert_eq!(svg, &render_bom_drawing(&design).svg, "the table is deterministic");

    let boxes = text_boxes(svg);
    assert_inside_canvas(svg, &boxes);
    assert_no_text_overlap(&boxes);
}

#[test]
fn bom_drawing_pdf_paginates_and_stays_deterministic() {
    let design = sample_design();
    let (contents, dropped, pages) = crate::render_bom_pdf(&design);
    assert!(contents.starts_with("%PDF-1.4"), "the artifact is a PDF");
    assert_eq!(dropped, 0, "the BOM cells are plain ASCII");
    assert_eq!(pages, 1, "seven rows fit one page");
    assert!(contents.contains("bill of materials"), "the page header names the artifact");
    assert!(contents.contains("Tj"), "the rows are written as text");
    let (again, _, _) = crate::render_bom_pdf(&design);
    assert_eq!(contents, again, "the PDF is byte-deterministic");

    let mut many = sample_design();
    many.sheets.truncate(1);
    many.sheets[0].nets.clear();
    many.sheets[0].components =
        (1..=30).map(|index| component(&format!("T{index:02}"), &format!("V{index}"), 10.0, 10.0, &["1"])).collect();
    let (contents, dropped, pages) = crate::render_bom_pdf(&many);
    assert_eq!(pages, 2, "thirty rows need two pages");
    assert_eq!(dropped, 0);
    assert!(contents.contains("(page 1 of 2)"), "the first page counts itself");
    assert!(contents.contains("(page 2 of 2)"), "the last page counts itself");
    assert!(contents.contains("T01"), "the first row lands on page one");
    assert!(contents.contains("T30"), "the last row lands on page two");
}

#[test]
fn net_xref_lists_members_and_sheet_numbers() {
    let design = sample_design();
    let drawing = render_net_xref(&design);
    let svg = &drawing.svg;
    assert!(svg.contains(r#"data-net-xref="true""#), "the sheet carries its group marker");
    assert!(svg.contains(r#"data-title-block="true""#), "the table carries the shared title block");
    assert!(svg.contains("NET CROSS REFERENCE"), "the table is named");
    assert!(svg.contains(">VCC</text>"), "a net name is listed");
    assert!(svg.contains("U1.VCC") && svg.contains("C1.2"), "nodes are grouped under the net");
    assert_eq!(drawing.nets, 6, "VCC, GND, IO1, LED, SPARE and SIG");
    assert_eq!(drawing.nodes, 14, "every distinct reference.pin member is listed");
    assert_eq!(drawing.pages, 1, "the sample fits one table page");
    assert_eq!(svg, &render_net_xref(&design).svg, "the table is deterministic");
    let boxes = text_boxes(svg);
    assert_inside_canvas(svg, &boxes);
    assert_no_text_overlap(&boxes);
}

#[test]
fn net_xref_wraps_long_member_lists_into_aligned_lines() {
    let mut design = sample_design();
    design.sheets.truncate(1);
    design.sheets[0].components = (1..=24).map(|index| component(&format!("U{index:02}"), "IO", 10.0, 10.0, &["1"])).collect();
    design.sheets[0].nets = vec![Net {
        id: "net-long".to_string(),
        name: "LONG_MEMBER_LIST".to_string(),
        nodes: (1..=24).map(|index| Node { reference: format!("U{index:02}"), pin: "1".to_string() }).collect(),
    }];
    let drawing = render_net_xref(&design);
    let nodes_cells = occurrences(&drawing.svg, "data-table-cell=\"true\"");
    assert!(nodes_cells >= 5, "the long member list wraps onto several text lines");
    assert!(drawing.svg.contains("U24.1"), "the last member survives the wrap");
    let boxes = text_boxes(&drawing.svg);
    assert_inside_canvas(&drawing.svg, &boxes);
    assert_no_text_overlap(&boxes);
}

#[test]
fn net_xref_pdf_paginates_and_stays_deterministic() {
    let design = sample_design();
    let (contents, dropped, pages, nets) = crate::render_net_xref_pdf(&design);
    assert!(contents.starts_with("%PDF-1.4"), "the artifact is a PDF");
    assert_eq!(dropped, 0, "the cross-reference cells are plain ASCII");
    assert_eq!(pages, 1, "the sample fits one page");
    assert_eq!(nets, 6, "every net is listed");
    assert!(contents.contains("net cross reference"), "the page header names the artifact");
    assert!(contents.contains("REFERENCE"), "the node column is named");
    let (again, _, _, _) = crate::render_net_xref_pdf(&design);
    assert_eq!(contents, again, "the PDF is byte-deterministic");

    let mut many = sample_design();
    many.sheets.truncate(1);
    many.sheets[0].components = (1..=80).map(|index| component(&format!("U{index:02}"), "IO", 10.0, 10.0, &["1"])).collect();
    many.sheets[0].nets = (1..=40)
        .map(|index| Net {
            id: format!("net-{index}"),
            name: format!("NET_{index:02}"),
            nodes: (1..=20).map(|member| Node { reference: format!("U{member:02}"), pin: "1".to_string() }).collect(),
        })
        .collect();
    let (_, dropped, pages, _) = crate::render_net_xref_pdf(&many);
    assert_eq!(dropped, 0);
    assert!(pages > 1, "a wrapped cross-reference needs more than one page");
}
