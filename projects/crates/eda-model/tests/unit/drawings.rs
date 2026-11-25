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

/// A design where two nets wire the same component pair, so every net name would
/// land on an identical label point without avoidance.
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

/// One parsed text element.
struct TextRun {
    x: f64,
    y: f64,
    size: f64,
    anchor: String,
    text: String,
    attributes: String,
}

/// One parsed straight line or polyline segment.
#[derive(Clone, Copy, Debug)]
struct Segment {
    from: [f64; 2],
    to: [f64; 2],
}

/// The content fragment of a drawing page plus its fit transform.
struct Page {
    content: String,
    translate: (f64, f64),
    scale: f64,
    content_size: (f64, f64),
}

/// Parses the shared page wrapper: the fitted content group and its transform.
fn page(svg: &str) -> Page {
    let group = svg.find("data-drawing=\"").expect("content group");
    let tag_start = svg[..group].rfind("<g ").expect("group start");
    let tag_end = tag_start + svg[tag_start..].find('>').expect("group tag end");
    let tag = &svg[tag_start..tag_end];
    let value = |key: &str| -> f64 {
        let at = tag.find(key).expect("attribute") + key.len();
        let rest = &tag[at..];
        rest[..rest.find('"').expect("value end")].parse().expect("number")
    };
    let scale = value("data-scale=\"");
    let sizes = &tag[tag.find("data-content=\"").expect("content size") + "data-content=\"".len()..];
    let sizes = &sizes[..sizes.find('"').expect("content size end")];
    let sizes: Vec<f64> = sizes.split_whitespace().map(|number| number.parse().expect("number")).collect();
    let transform = &tag[tag.find("transform=\"").expect("transform") + "transform=\"".len()..];
    let transform = &transform[..transform.find('"').expect("transform end")];
    let numbers: Vec<f64> = transform
        .split(|character: char| !character.is_ascii_digit() && character != '.' && character != '-')
        .filter(|token| !token.is_empty())
        .map(|token| token.parse().expect("number"))
        .collect();
    let title = svg.find("data-title-block=\"true\"").expect("title block");
    let content_text = &svg[tag_end + 1..title];
    let content_text = content_text.strip_suffix("  </g>\n").unwrap_or(content_text).to_string();
    Page { content: content_text, translate: (numbers[0], numbers[1]), scale, content_size: (sizes[0], sizes[1]) }
}

/// Parses every `<text>` element of a fragment.
fn text_runs(fragment: &str) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut rest = fragment;
    while let Some(at) = rest.find("<text ") {
        let tag_end = at + rest[at..].find('>').expect("tag end");
        let tag = &rest[at..tag_end];
        let value = |key: &str| -> f64 {
            let at = tag.find(key).expect("attribute") + key.len();
            let rest = &tag[at..];
            rest[..rest.find('"').expect("value end")].parse().expect("number")
        };
        let content = &rest[tag_end + 1..];
        let text = &content[..content.find('<').expect("text end")];
        let anchor = if tag.contains("text-anchor=\"middle\"") {
            "middle"
        }
        else if tag.contains("text-anchor=\"end\"") {
            "end"
        }
        else {
            "start"
        };
        runs.push(TextRun {
            x: value(" x=\""),
            y: value(" y=\""),
            size: value("font-size=\""),
            anchor: anchor.to_string(),
            text: text.to_string(),
            attributes: tag.to_string(),
        });
        rest = &content[text.len() + 1..];
    }
    runs
}

/// The estimated painted box of a text run under a translate-and-scale fit.
fn text_box(run: &TextRun, transform: (f64, f64), scale: f64) -> [f64; 4] {
    let width = run.text.chars().count() as f64 * run.size * 0.6 * scale;
    let height = run.size * scale;
    let x = transform.0 + run.x * scale;
    let y = transform.1 + run.y * scale;
    let left = match run.anchor.as_str() {
        "middle" => x - width / 2.0,
        "end" => x - width,
        _ => x,
    };
    [left, y - height * 0.8, left + width, y]
}

/// Parses a numeric attribute from an SVG tag.
fn attribute(tag: &str, key: &str) -> Option<f64> {
    let at = tag.find(key)? + key.len();
    let rest = &tag[at..];
    rest[..rest.find('"')?].parse().ok()
}

/// Parses every line or polyline segment of a fragment.
fn segments(fragment: &str) -> Vec<Segment> {
    let mut found = Vec::new();
    let mut rest = fragment;
    while let Some(at) = rest.find("<line ") {
        let tag_end = at + rest[at..].find('>').expect("line end");
        let tag = &rest[at..tag_end];
        if let (Some(x1), Some(y1), Some(x2), Some(y2)) =
            (attribute(tag, " x1=\""), attribute(tag, " y1=\""), attribute(tag, " x2=\""), attribute(tag, " y2=\""))
        {
            found.push(Segment { from: [x1, y1], to: [x2, y2] });
        }
        rest = &rest[tag_end..];
    }
    let mut rest = fragment;
    while let Some(at) = rest.find("<polyline ") {
        let tag_end = at + rest[at..].find('>').expect("polyline end");
        let tag = &rest[at..tag_end];
        if let Some(points) = tag.find("points=\"").map(|start| {
            let start = start + "points=\"".len();
            let end = start + tag[start..].find('"').expect("points end");
            &tag[start..end]
        }) {
            let points: Vec<[f64; 2]> = points
                .split_whitespace()
                .filter_map(|pair| {
                    let mut parts = pair.split(',');
                    Some([parts.next()?.parse().ok()?, parts.next()?.parse().ok()?])
                })
                .collect();
            for pair in points.windows(2) {
                found.push(Segment { from: pair[0], to: pair[1] });
            }
        }
        rest = &rest[tag_end..];
    }
    found
}

/// Every `<rect>` that directly follows a group carrying `marker`.
fn group_rects(fragment: &str, marker: &str) -> Vec<[f64; 4]> {
    let mut rects = Vec::new();
    let mut rest = fragment;
    while let Some(at) = rest.find(marker) {
        let after = &rest[at..];
        let Some(rect_at) = after.find("<rect ")
        else {
            break;
        };
        let rect_end = rect_at + after[rect_at..].find('>').expect("rect end");
        let tag = &after[rect_at..rect_end];
        if let (Some(x), Some(y), Some(width), Some(height)) =
            (attribute(tag, " x=\""), attribute(tag, " y=\""), attribute(tag, " width=\""), attribute(tag, " height=\""))
        {
            rects.push([x, y, x + width, y + height]);
        }
        rest = &after[rect_end..];
    }
    rects
}

/// Every rect carrying `marker` anywhere in its tag.
fn rects_with(fragment: &str, marker: &str) -> Vec<[f64; 4]> {
    let mut rects = Vec::new();
    let mut rest = fragment;
    while let Some(at) = rest.find("<rect ") {
        let rect_end = at + rest[at..].find('>').expect("rect end");
        let tag = &rest[at..rect_end];
        if tag.contains(marker)
            && let (Some(x), Some(y), Some(width), Some(height)) =
                (attribute(tag, " x=\""), attribute(tag, " y=\""), attribute(tag, " width=\""), attribute(tag, " height=\""))
        {
            rects.push([x, y, x + width, y + height]);
        }
        rest = &rest[rect_end..];
    }
    rects
}

fn disjoint(left: &[f64; 4], right: &[f64; 4]) -> bool {
    left[2] <= right[0] + 0.01 || right[2] <= left[0] + 0.01 || left[3] <= right[1] + 0.01 || right[3] <= left[1] + 0.01
}

fn assert_no_overlap(boxes: &[[f64; 4]], what: &str) {
    for (index, left) in boxes.iter().enumerate() {
        for right in &boxes[index + 1..] {
            assert!(disjoint(left, right), "{what} overlap: {left:?} vs {right:?}");
        }
    }
}

fn strictly_inside(point: [f64; 2], rect: &[f64; 4]) -> bool {
    point[0] > rect[0] + 0.05 && point[0] < rect[2] - 0.05 && point[1] > rect[1] + 0.05 && point[1] < rect[3] - 0.05
}

/// True when the interior of a segment passes through a rect.
fn crosses(segment: Segment, rect: &[f64; 4]) -> bool {
    (1..=9).any(|step| {
        let t = step as f64 / 10.0;
        strictly_inside(
            [segment.from[0] + (segment.to[0] - segment.from[0]) * t, segment.from[1] + (segment.to[1] - segment.from[1]) * t],
            rect,
        )
    })
}

/// The drawable width and height of the A4 page.
fn drawable() -> (f64, f64) {
    (297.0 - 2.0 * 297.0 * 0.08, 210.0 - 2.0 * 210.0 * 0.08 - 22.0)
}

fn view_box(svg: &str) -> [f64; 4] {
    let value = &svg[svg.find("viewBox=\"").expect("viewBox") + "viewBox=\"".len()..];
    let value = &value[..value.find('"').expect("viewBox end")];
    let numbers: Vec<f64> = value.split_whitespace().map(|number| number.parse().expect("number")).collect();
    [numbers[0], numbers[1], numbers[2], numbers[3]]
}

#[test]
fn every_drawing_uses_one_a4_landscape_page() {
    let design = sample_design();
    let drawings = [
        render_block_diagram(&design).svg,
        render_sheet_overview(&design).svg,
        render_pinout_chart(&design).svg,
        render_symbol_library(&design).svg,
        render_net_topology(&design).svg,
        render_power_distribution(&design).svg,
        render_bom_drawing(&design).svg,
        render_net_xref(&design).svg,
    ];
    for svg in &drawings {
        assert_eq!(view_box(svg), [0.0, 0.0, 297.0, 210.0], "every drawing is A4 landscape");
        assert_eq!(occurrences(svg, r#"data-title-block="true""#), 1, "exactly one fixed title block");
        assert_eq!(occurrences(svg, r#"data-drawing=""#), 1, "one fitted content group");
    }
}

#[test]
fn fitted_content_covers_at_least_sixty_percent_of_the_drawable_area() {
    let design = sample_design();
    let (draw_width, draw_height) = drawable();
    let drawings = [
        ("block", render_block_diagram(&design).svg),
        ("sheet overview", render_sheet_overview(&design).svg),
        ("pinout", render_pinout_chart(&design).svg),
        ("symbol library", render_symbol_library(&design).svg),
        ("net topology", render_net_topology(&design).svg),
        ("power", render_power_distribution(&design).svg),
        ("bom", render_bom_drawing(&design).svg),
        ("net xref", render_net_xref(&design).svg),
    ];
    for (name, svg) in &drawings {
        let page = page(svg);
        let width = page.content_size.0 * page.scale;
        let height = page.content_size.1 * page.scale;
        assert!(
            width >= draw_width * 0.6 - 0.01,
            "{name} content covers only {:.0}% of the drawable width",
            width / draw_width * 100.0
        );
        assert!(
            height >= draw_height * 0.6 - 0.01,
            "{name} content covers only {:.0}% of the drawable height",
            height / draw_height * 100.0
        );
    }
}

#[test]
fn every_text_box_stays_inside_the_page() {
    let design = sample_design();
    let drawings = [
        render_block_diagram(&design).svg,
        render_sheet_overview(&design).svg,
        render_pinout_chart(&design).svg,
        render_symbol_library(&design).svg,
        render_net_topology(&design).svg,
        render_power_distribution(&design).svg,
        render_bom_drawing(&design).svg,
        render_net_xref(&design).svg,
    ];
    for svg in &drawings {
        let page = page(svg);
        let mut boxes: Vec<[f64; 4]> =
            text_runs(&page.content).iter().map(|run| text_box(run, page.translate, page.scale)).collect();
        let title = &svg[svg.find("data-title-block=\"true\"").expect("title block")..];
        boxes.extend(text_runs(title).iter().map(|run| text_box(run, (0.0, 0.0), 1.0)));
        for bounds in &boxes {
            assert!(
                bounds[0] >= -0.01 && bounds[1] >= -0.01 && bounds[2] <= 297.01 && bounds[3] <= 210.01,
                "text leaves the A4 page: {bounds:?}"
            );
        }
    }
}

#[test]
fn painted_text_keeps_the_readable_size_floor() {
    let design = sample_design();
    let drawings = [
        ("block", render_block_diagram(&design).svg),
        ("pinout", render_pinout_chart(&design).svg),
        ("net topology", render_net_topology(&design).svg),
        ("power", render_power_distribution(&design).svg),
        ("bom", render_bom_drawing(&design).svg),
        ("net xref", render_net_xref(&design).svg),
    ];
    for (name, svg) in &drawings {
        let page = page(svg);
        for run in text_runs(&page.content) {
            let painted = run.size * page.scale;
            assert!(
                painted >= 2.35,
                "{name} paints `{}` at {painted:.2} mm, below the 6.5 pt floor",
                run.text
            );
        }
    }
}

#[test]
fn title_block_text_fits_its_fixed_cell() {
    let design = sample_design();
    let svg = render_block_diagram(&design).svg;
    let title_start = svg.find("data-title-block=\"true\"").expect("title block");
    let title = &svg[title_start..];
    let columns = [0.0, 132.0, 210.0, 250.0, 297.0];
    for run in text_runs(title) {
        let bounds = text_box(&run, (0.0, 0.0), 1.0);
        let cell = columns.windows(2).find(|pair| run.x >= pair[0] && run.x < pair[1]).expect("text starts in a cell");
        assert!(bounds[2] <= cell[1] - 0.5, "title text `{}` overflows its cell: {} > {}", run.text, bounds[2], cell[1]);
        assert!(run.size >= 2.4, "title text keeps the readable floor");
    }
}

#[test]
fn block_diagram_routes_chains_around_boxes_and_places_labels_apart() {
    let design = sample_design();
    let drawing = render_block_diagram(&design);
    let page = page(&drawing.svg);

    assert_eq!(drawing.nodes, 8, "one node per component");
    assert_eq!(drawing.nets, 4, "VCC, GND, IO1 and LED have multiple members");
    assert_eq!(drawing.edges, 6, "2 + 2 + 1 + 1 segments");
    assert!(page.content.contains(r#"data-block-diagram="true""#));

    let boxes = group_rects(&page.content, "data-block=\"");
    assert_eq!(boxes.len(), 8, "every component paints one box");
    assert_no_overlap(&boxes, "component boxes");
    for segment in segments(&page.content) {
        for rect in &boxes {
            assert!(!crosses(segment, rect), "edge {segment:?} crosses component box {rect:?}");
        }
    }

    let runs = text_runs(&page.content);
    let labels: Vec<TextRun> = runs.into_iter().filter(|run| run.attributes.contains("data-net-name")).collect();
    assert_eq!(labels.len(), 4, "each multi-member net is named once");
    let label_boxes: Vec<[f64; 4]> = labels.iter().map(|run| text_box(run, (0.0, 0.0), 1.0)).collect();
    assert_no_overlap(&label_boxes, "net labels");
    for label in &label_boxes {
        for rect in &boxes {
            assert!(disjoint(label, rect), "net label {label:?} covers a component box {rect:?}");
        }
    }
    assert_eq!(drawing.svg, render_block_diagram(&design).svg, "the layout is deterministic");
}

#[test]
fn block_diagram_keeps_colliding_labels_apart() {
    let design = star_design();
    let drawing = render_block_diagram(&design);
    let page = page(&drawing.svg);
    let labels: Vec<[f64; 4]> = text_runs(&page.content)
        .iter()
        .filter(|run| run.attributes.contains("data-net-name"))
        .map(|run| text_box(run, (0.0, 0.0), 1.0))
        .collect();
    assert_eq!(labels.len(), 4);
    assert_no_overlap(&labels, "labels that share an anchor");
    let (tx, ty) = page.translate;
    for label in &labels {
        let page_label =
            [tx + label[0] * page.scale, ty + label[1] * page.scale, tx + label[2] * page.scale, ty + label[3] * page.scale];
        assert!(
            page_label[0] >= 0.0 && page_label[1] >= 0.0 && page_label[2] <= 297.0 && page_label[3] <= 210.0,
            "a displaced label leaves the page: {page_label:?}"
        );
    }
}

#[test]
fn net_topology_gives_every_net_its_own_trunk_and_readable_box() {
    let design = sample_design();
    let drawing = render_net_topology(&design);
    let page = page(&drawing.svg);
    assert_eq!(drawing.components, 8);
    assert_eq!(drawing.nets, 6);
    assert_eq!(drawing.edges, 12, "one incidence per component-net pair");
    assert!(page.content.contains(r#"data-net-node="SPARE""#), "single-member nets get a box too");

    let trunk_x = {
        let mut values = Vec::new();
        let mut rest = page.content.as_str();
        while let Some(at) = rest.find("data-trunk=\"true\"") {
            let tag_start = rest[..at].rfind("<line ").expect("trunk line");
            let tag_end = tag_start + rest[tag_start..].find('>').expect("trunk end");
            values.push(attribute(&rest[tag_start..tag_end], " x1=\"").expect("x"));
            rest = &rest[tag_end..];
        }
        values
    };
    assert_eq!(trunk_x.len(), 6, "one trunk per net");
    for (index, x) in trunk_x.iter().enumerate() {
        assert!(trunk_x[index + 1..].iter().all(|other| (*other - *x).abs() > 0.5), "trunks must not be collinear");
    }

    let runs = text_runs(&page.content);
    let boxes: Vec<[f64; 4]> =
        runs.iter().filter(|run| run.attributes.contains("data-net-name")).map(|run| text_box(run, (0.0, 0.0), 1.0)).collect();
    assert_eq!(boxes.len(), 6);
    assert_no_overlap(&boxes, "net boxes");
    let components = group_rects(&page.content, "data-block=\"");
    assert_eq!(components.len(), 8);
    for label in &boxes {
        for rect in &components {
            assert!(disjoint(label, rect), "net label covers a component box");
        }
    }
    assert_eq!(drawing.svg, render_net_topology(&design).svg, "the graph is deterministic");
}

#[test]
fn power_distribution_connects_every_tap_to_a_rail_without_dangling_runs() {
    let design = sample_design();
    let drawing = render_power_distribution(&design);
    let page = page(&drawing.svg);
    assert_eq!(drawing.power_nets, 1, "VCC is the only power net");
    assert_eq!(drawing.ground_nets, 1, "GND is the only ground net");
    assert_eq!(drawing.connections, 6, "VCC and GND have three members each");
    assert!(page.content.contains(r#"data-rail="power" data-net="VCC""#));
    assert!(page.content.contains(r#"data-rail="ground" data-net="GND""#));

    let boxes = group_rects(&page.content, "data-block=\"");
    assert_eq!(boxes.len(), 8);
    let rails = rects_with(&page.content, "data-rail=\"");
    assert_eq!(rails.len(), 2);
    for segment in segments(&page.content) {
        for rect in &boxes {
            assert!(!crosses(segment, rect), "rail wiring crosses a component box: {segment:?} vs {rect:?}");
        }
    }

    let mut risers = 0;
    let mut rest = page.content.as_str();
    while let Some(at) = rest.find("data-riser=\"") {
        let tag_start = rest[..at].rfind("<line ").expect("riser line");
        let tag_end = tag_start + rest[tag_start..].find('>').expect("riser end");
        let tag = &rest[tag_start..tag_end];
        let kind = &rest[at + "data-riser=\"".len()..];
        let kind = &kind[..kind.find('"').expect("kind end")];
        let x = attribute(tag, " x1=\"").expect("x");
        let y1 = attribute(tag, " y1=\"").expect("y1");
        let y2 = attribute(tag, " y2=\"").expect("y2");
        let on_rail = rails.iter().any(|rail| {
            x >= rail[0] - 0.01
                && x <= rail[2] + 0.01
                && (kind == "power" && (y1 - rail[3]).abs() < 0.1 || kind == "ground" && (y2 - rail[1]).abs() < 0.1)
        });
        assert!(on_rail, "a {kind} riser at ({x}, {y1}..{y2}) does not land on its rail");
        risers += 1;
        rest = &rest[tag_end..];
    }
    assert!(risers >= 2, "each column that carries members gets a riser");

    let boxes_text = text_runs(&page.content);
    let labels: Vec<[f64; 4]> = boxes_text.iter().map(|run| text_box(run, (0.0, 0.0), 1.0)).collect();
    assert_no_overlap(&labels, "rail and component labels");
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
    assert_eq!(drawing.power_nets, 0);
    assert_eq!(drawing.ground_nets, 0);
    assert!(drawing.svg.contains("no power or ground nets detected"));
}

#[test]
fn symbol_library_paints_a_complete_grid_of_readable_cells() {
    let design = sample_design();
    let drawing = render_symbol_library(&design);
    let page = page(&drawing.svg);
    assert_eq!(drawing.symbols, 6, "R, C, U, D, J and Q symbols");
    assert_eq!(drawing.components, 8, "deduplicated from eight placed parts");
    assert!(page.content.contains(r#"data-symbol="R|1|2""#));
    assert!(page.content.contains(r#"data-symbol="U|VCC|GND|IO1|IO2""#));
    assert_eq!(occurrences(&page.content, r#"data-symbol=""#), 6, "one cell per distinct symbol");

    let cells = group_rects(&page.content, "data-symbol=\"");
    assert_eq!(cells.len(), 6);
    assert_no_overlap(&cells, "symbol cells");
    for cell in &cells {
        assert!(cell[2] - cell[0] > 20.0 && cell[3] - cell[1] > 20.0, "a cell is too small: {cell:?}");
    }
    assert_eq!(drawing.svg, render_symbol_library(&design).svg, "the library is deterministic");
}

#[test]
fn sheet_overview_grids_every_sheet_under_one_title_block() {
    let design = sample_design();
    let drawing = render_sheet_overview(&design);
    let svg = &drawing.svg;
    assert!(svg.contains(r#"data-sheet-overview="true""#));
    assert!(svg.contains(r#"data-sheet="sheet-1""#) && svg.contains(r#"data-sheet="sheet-2""#));
    assert_eq!(occurrences(svg, r#"data-title-block="true""#), 1);
    assert_eq!((drawing.columns, drawing.rows), (2, 1));
    assert!(svg.contains(">Power<") && svg.contains(">Output<"));
    assert_eq!(svg, &render_sheet_overview(&design).svg);
}

#[test]
fn pinout_chart_maps_pins_to_nets_and_marks_no_connects() {
    let design = sample_design();
    let drawing = render_pinout_chart(&design);
    let svg = &drawing.svg;
    assert!(svg.contains(r#"data-pinout-chart="true""#));
    assert!(svg.contains(r#"data-ref="U1""#));
    assert!(svg.contains(r#"data-pin="VCC""#));
    assert!(svg.contains(">VCC, VCC<") || svg.contains(r#">VCC</text>"#));
    let j1 = svg.find(r#"data-ref="J1""#).expect("J1 block");
    let j1_block = &svg[j1..];
    assert!(j1_block.contains(r#"data-no-connect="true""#));
    assert!(j1_block.contains(">NC</text>"));
    assert!(!svg.contains("—"));
    let q1 = svg.find(r#"data-ref="Q1""#).expect("Q1 block");
    assert!(svg[q1..].contains(r#"data-pin="E""#));
    assert_eq!(drawing.components, 8);
    assert_eq!(drawing.rows, 19, "4+2+2+2+2+2+2+3 pin rows");
    assert_eq!(svg, &render_pinout_chart(&design).svg);
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
fn bom_drawing_uses_the_full_width_and_drops_the_empty_footprint_column() {
    let design = sample_design();
    let drawing = render_bom_drawing(&design);
    let page = page(&drawing.svg);
    assert_eq!(drawing.rows, 7);
    assert_eq!(drawing.components, 8);
    assert!(!page.content.contains(">FOOTPRINT<"), "an entirely empty column is not drawn");
    assert_eq!(occurrences(&page.content, "data-bom-cell="), 28, "four painted columns over seven rows");
    assert!(page.content.contains(">R1, R3<"), "the two 10k parts share one designator cell");
    let width = page.content_size.0 * page.scale;
    assert!(width >= drawable().0 * 0.85, "the table spans the page width");
    assert_eq!(drawing.svg, render_bom_drawing(&design).svg, "the table is deterministic");
}

#[test]
fn net_xref_fills_the_page_and_lists_every_member() {
    let design = sample_design();
    let drawing = render_net_xref(&design);
    let page = page(&drawing.svg);
    assert!(page.content.contains(r#"data-net-xref="true""#));
    assert!(page.content.contains("NET CROSS REFERENCE"));
    assert!(page.content.contains(">VCC</text>"));
    assert!(page.content.contains("U1.VCC") && page.content.contains("C1.2"));
    assert_eq!(drawing.nets, 6);
    assert_eq!(drawing.nodes, 14);
    assert_eq!(drawing.pages, 1);
    let width = page.content_size.0 * page.scale;
    assert!(width >= drawable().0 * 0.85, "the table spans the page width");
    let boxes = text_runs(&page.content).iter().map(|run| text_box(run, (0.0, 0.0), 1.0)).collect::<Vec<_>>();
    assert_no_overlap(&boxes, "table text");
    assert_eq!(drawing.svg, render_net_xref(&design).svg);
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
    let page = page(&drawing.svg);
    assert!(occurrences(&page.content, "data-table-cell=\"true\"") > 3, "the long member list wraps");
    assert!(page.content.contains("U24.1"));
    let boxes = text_runs(&page.content).iter().map(|run| text_box(run, (0.0, 0.0), 1.0)).collect::<Vec<_>>();
    assert_no_overlap(&boxes, "wrapped table text");
}

#[test]
fn empty_designs_still_paint_complete_documents() {
    let mut design = sample_design();
    for sheet in &mut design.sheets {
        sheet.components.clear();
        sheet.nets.clear();
        sheet.wires.clear();
    }
    design.sheets.clear();
    for drawing in [
        render_block_diagram(&design).svg,
        render_net_topology(&design).svg,
        render_power_distribution(&design).svg,
        render_bom_drawing(&design).svg,
        render_net_xref(&design).svg,
        render_symbol_library(&design).svg,
        render_sheet_overview(&design).svg,
    ] {
        assert!(drawing.ends_with("</svg>\n"), "an empty design still closes the document");
        assert_eq!(view_box(&drawing), [0.0, 0.0, 297.0, 210.0]);
        assert!(drawing.contains(r#"data-title-block="true""#));
    }
}

#[test]
fn bom_drawing_pdf_paginates_and_stays_deterministic() {
    let design = sample_design();
    let (contents, dropped, pages) = crate::render_bom_pdf(&design);
    assert!(contents.starts_with("%PDF-1.4"));
    assert_eq!(dropped, 0);
    assert_eq!(pages, 1);
    assert!(contents.contains("bill of materials"));
    assert!(contents.contains("Tj"));
    let (again, _, _) = crate::render_bom_pdf(&design);
    assert_eq!(contents, again);

    let mut many = sample_design();
    many.sheets.truncate(1);
    many.sheets[0].nets.clear();
    many.sheets[0].components =
        (1..=30).map(|index| component(&format!("T{index:02}"), &format!("V{index}"), 10.0, 10.0, &["1"])).collect();
    let (contents, dropped, pages) = crate::render_bom_pdf(&many);
    assert_eq!(pages, 2);
    assert_eq!(dropped, 0);
    assert!(contents.contains("(page 1 of 2)"));
    assert!(contents.contains("(page 2 of 2)"));
    assert!(contents.contains("T01"));
    assert!(contents.contains("T30"));
}

#[test]
fn net_xref_pdf_paginates_and_stays_deterministic() {
    let design = sample_design();
    let (contents, dropped, pages, nets) = crate::render_net_xref_pdf(&design);
    assert!(contents.starts_with("%PDF-1.4"));
    assert_eq!(dropped, 0);
    assert_eq!(pages, 1);
    assert_eq!(nets, 6);
    assert!(contents.contains("net cross reference"));
    assert!(contents.contains("REFERENCE"));
    let (again, _, _, _) = crate::render_net_xref_pdf(&design);
    assert_eq!(contents, again);

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
    assert!(pages > 1);
}
