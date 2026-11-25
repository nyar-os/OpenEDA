use crate::{
    Component, Design, DesignRules, Junction, LabelScope, Net, NetLabel, NoConnect, Node, Sheet, TextLabel, Wire,
    drawings::{
        render_block_diagram, render_bom_drawing, render_erc, render_hierarchy, render_net_topology, render_net_xref,
        render_pinout_chart, render_power_distribution, render_sheet_overview, render_symbol_library,
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
        render_hierarchy(&design).svg,
        render_erc(&design).svg,
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
        ("hierarchy", render_hierarchy(&design).svg),
        ("erc", render_erc(&design).svg),
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
        render_hierarchy(&design).svg,
        render_erc(&design).svg,
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
        ("hierarchy", render_hierarchy(&design).svg),
        ("erc", render_erc(&design).svg),
    ];
    for (name, svg) in &drawings {
        let page = page(svg);
        for run in text_runs(&page.content) {
            let painted = run.size * page.scale;
            assert!(painted >= 2.35, "{name} paints `{}` at {painted:.2} mm, below the 6.5 pt floor", run.text);
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
fn hierarchy_tree_places_nodes_without_overlap_and_connects_them() {
    let design = sample_design();
    let drawing = render_hierarchy(&design);
    let page = page(&drawing.svg);
    assert_eq!(drawing.nodes, 2, "two flat sheets when no hierarchy symbols exist");
    let nodes = rects_with(&page.content, "data-tree-node=\"true\"");
    assert_eq!(nodes.len(), 3, "one root plus two children");
    assert_no_overlap(&nodes, "tree nodes");
    let connectors = segments(&page.content);
    assert!(!connectors.is_empty(), "the tree routes connectors");
    for segment in &connectors {
        for node in &nodes {
            assert!(!crosses(*segment, node), "a tree connector crosses a node: {segment:?} vs {node:?}");
        }
    }
    let texts: Vec<[f64; 4]> = text_runs(&page.content).iter().map(|run| text_box(run, (0.0, 0.0), 1.0)).collect();
    assert_no_overlap(&texts, "tree text");
    assert_eq!(drawing.svg, render_hierarchy(&design).svg);
}

#[test]
fn erc_table_reports_findings_or_prints_no_findings() {
    // A duplicated reference is an `erc` finding without coordinates.
    let mut broken = sample_design();
    broken.sheets.truncate(1);
    broken.sheets[0].components.push(component("U1", "DUPLICATE", 10.0, 10.0, &["1"]));
    let drawing = render_erc(&broken);
    let findings_page = page(&drawing.svg);
    assert!(drawing.findings > 0, "a duplicate reference is reported");
    assert!(findings_page.content.contains(r#"data-erc="true""#));
    assert!(
        findings_page.content.contains("SEVERITY")
            && findings_page.content.contains("OBJECT")
            && findings_page.content.contains("DESCRIPTION")
    );
    assert!(!findings_page.content.contains("POSITION"), "a column with no values is not drawn");

    let clean = {
        let mut design = sample_design();
        design.sheets.clear();
        design
    };
    let clean = render_erc(&clean);
    assert_eq!(clean.findings, 0);
    assert!(clean.svg.contains(">No findings<"), "an empty report says so");
    assert!(!clean.svg.contains("SEVERITY"), "no empty table is drawn");

    // A long finding list stays on one page at a readable size and summarises
    // the remainder instead of shrinking every row.
    let full = render_erc(&sample_design());
    let full_page = page(&full.svg);
    assert!(full_page.scale > 0.95, "the findings page is not shrunk into unreadability");
    if full.findings > 20 {
        assert!(full_page.content.contains("more finding(s) not shown"), "the remainder is counted");
    }
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
        render_hierarchy(&design).svg,
        render_erc(&design).svg,
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

/// One affine SVG transform: `x' = a*x + c*y + e`, `y' = b*x + d*y + f`.
#[derive(Clone, Copy)]
struct Affine {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl Affine {
    const IDENTITY: Affine = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    /// Composes two transforms so `next` applies first, then `self`.
    fn then(self, next: Affine) -> Affine {
        Affine {
            a: self.a * next.a + self.c * next.b,
            b: self.b * next.a + self.d * next.b,
            c: self.a * next.c + self.c * next.d,
            d: self.b * next.c + self.d * next.d,
            e: self.a * next.e + self.c * next.f + self.e,
            f: self.b * next.e + self.d * next.f + self.f,
        }
    }

    fn point(self, point: [f64; 2]) -> [f64; 2] {
        [self.a * point[0] + self.c * point[1] + self.e, self.b * point[0] + self.d * point[1] + self.f]
    }

    /// Uniform length scale of the composed transform.
    fn scale(self) -> f64 {
        (self.a * self.a + self.b * self.b).sqrt()
    }
}

/// Parses one SVG `transform` attribute, including nested function lists.
fn parse_transform(text: &str) -> Affine {
    let mut result = Affine::IDENTITY;
    let mut rest = text;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim();
        let name = name.rsplit(|character: char| !character.is_ascii_alphabetic()).next().unwrap_or("");
        let Some(close) = rest[open..].find(')')
        else {
            break;
        };
        let body = &rest[open + 1..open + close];
        let numbers: Vec<f64> = body
            .split(|character: char| character == ',' || character.is_whitespace())
            .filter(|token| !token.is_empty())
            .map(|token| token.parse().expect("transform number"))
            .collect();
        let step = match name {
            "translate" => Affine { e: numbers[0], f: numbers.get(1).copied().unwrap_or(0.0), ..Affine::IDENTITY },
            "scale" => {
                let x = numbers[0];
                let y = numbers.get(1).copied().unwrap_or(x);
                Affine { a: x, d: y, ..Affine::IDENTITY }
            }
            "rotate" if !numbers.is_empty() => {
                let radians = numbers[0].to_radians();
                let (sin, cos) = radians.sin_cos();
                if numbers.len() >= 3 {
                    let (cx, cy) = (numbers[1], numbers[2]);
                    Affine { a: cos, b: sin, c: -sin, d: cos, e: cx - cos * cx + sin * cy, f: cy - sin * cx - cos * cy }
                }
                else {
                    Affine { a: cos, b: sin, c: -sin, d: cos, ..Affine::IDENTITY }
                }
            }
            _ => Affine::IDENTITY,
        };
        result = result.then(step);
        rest = &rest[open + close + 1..];
    }
    result
}

/// Reads one string attribute from an SVG tag.
fn tag_text<'a>(tag: &'a str, key: &str) -> Option<&'a str> {
    let at = tag.find(key)? + key.len();
    let rest = &tag[at..];
    Some(&rest[..rest.find('"')?])
}

/// One text element with its painted bounding box on the page.
#[derive(Debug)]
struct PlacedText {
    text: String,
    bounds: [f64; 4],
    /// Effective glyph height in page units after all enclosing scales.
    size: f64,
}

/// Walks an SVG document, composes every enclosing `translate`, `scale` and
/// `rotate` transform and returns the page-space box of every `<text>` element.
fn placed_texts(svg: &str) -> Vec<PlacedText> {
    let mut stack = vec![Affine::IDENTITY];
    let mut placed = Vec::new();
    let mut rest = svg;
    while let Some(offset) = rest.find('<') {
        rest = &rest[offset..];
        if rest.starts_with("</g") {
            if stack.len() > 1 {
                stack.pop();
            }
            rest = &rest[rest.find('>').expect("group end") + 1..];
            continue;
        }
        let Some(tag_end) = rest.find('>')
        else {
            break;
        };
        let tag = &rest[..tag_end];
        if tag.starts_with("<g ") || tag == "<g" {
            let local = tag_text(tag, " transform=\"").map(parse_transform).unwrap_or(Affine::IDENTITY);
            let parent = *stack.last().expect("transform stack");
            stack.push(parent.then(local));
            rest = &rest[tag_end + 1..];
        }
        else if tag.starts_with("<text ") {
            let content = &rest[tag_end + 1..];
            let content_end = content.find('<').expect("text end");
            let text = &content[..content_end];
            let x: f64 = tag_text(tag, " x=\"").expect("text x").parse().expect("text x number");
            let y: f64 = tag_text(tag, " y=\"").expect("text y").parse().expect("text y number");
            let size: f64 = tag_text(tag, "font-size=\"").expect("text size").parse().expect("text size number");
            let anchor = tag_text(tag, "text-anchor=\"").unwrap_or("start");
            let width = text.chars().count() as f64 * size * 0.6;
            let left = match anchor {
                "middle" => x - width / 2.0,
                "end" => x - width,
                _ => x,
            };
            let local = tag_text(tag, " transform=\"").map(parse_transform).unwrap_or(Affine::IDENTITY);
            let total = stack.last().expect("transform stack").then(local);
            let corners = [
                total.point([left, y - size * 0.75]),
                total.point([left + width, y - size * 0.75]),
                total.point([left + width, y + size * 0.25]),
                total.point([left, y + size * 0.25]),
            ];
            let min_x = corners.iter().map(|point| point[0]).fold(f64::INFINITY, f64::min);
            let max_x = corners.iter().map(|point| point[0]).fold(f64::NEG_INFINITY, f64::max);
            let min_y = corners.iter().map(|point| point[1]).fold(f64::INFINITY, f64::min);
            let max_y = corners.iter().map(|point| point[1]).fold(f64::NEG_INFINITY, f64::max);
            placed.push(PlacedText {
                text: text.to_string(),
                bounds: [min_x, min_y, max_x, max_y],
                size: size * total.scale(),
            });
            rest = &content[content_end..];
        }
        else {
            rest = &rest[tag_end + 1..];
        }
    }
    placed
}

/// Asserts that every painted text stays inside the given page frame.
fn assert_texts_inside(texts: &[PlacedText], frame: [f64; 4], what: &str) {
    for placed in texts {
        let bounds = placed.bounds;
        assert!(
            bounds[0] >= frame[0] - 0.05
                && bounds[1] >= frame[1] - 0.05
                && bounds[2] <= frame[2] + 0.05
                && bounds[3] <= frame[3] + 0.05,
            "{what}: `{}` leaves the page frame {frame:?} at {bounds:?}",
            placed.text
        );
    }
}

/// Asserts that no two painted texts share any area.
fn assert_texts_apart(texts: &[PlacedText], what: &str) {
    for (index, left) in texts.iter().enumerate() {
        for right in &texts[index + 1..] {
            assert!(
                disjoint(&left.bounds, &right.bounds),
                "{what}: `{}` {:?} overlaps `{}` {:?}",
                left.text,
                left.bounds,
                right.text,
                right.bounds
            );
        }
    }
}

/// Pins that sit above a wire so the net name has to move clear of them.
fn pin(reference: &str, pin: &str) -> Node {
    Node { reference: reference.to_string(), pin: pin.to_string() }
}

/// Wires, labels and rotated parts modelled on a real schematic, including two
/// passives stacked close enough that their default reference/value lines would
/// collide.
fn crowded_design() -> Design {
    let sheet_one = Sheet {
        id: "sheet-1".to_string(),
        name: "Power".to_string(),
        components: vec![
            component("U1", "MCU", 60.0, 40.0, &["VCC", "GND", "PA0", "PA1", "PB0", "PB1"]),
            component("R1", "10k", 95.0, 18.0, &["1", "2"]),
            component("R6", "2k2", 95.0, 26.0, &["1", "2"]),
            component("R2", "330", 95.0, 62.0, &["1", "2"]),
            component("C1", "100n", 25.0, 40.0, &["1", "2"]),
            component("C2", "10u+", 25.0, 72.0, &["1", "2"]),
            component("D1", "LED", 130.0, 18.0, &["A", "K"]),
            component("L1", "4u7", 130.0, 62.0, &["1", "2"]),
        ],
        nets: vec![Net { id: "net-1".to_string(), name: "PB0".to_string(), nodes: vec![pin("U1", "PB0")] }],
        wires: vec![
            Wire { id: "w1".to_string(), net: Some("VCC".to_string()), from: [10.0, 10.0], to: [150.0, 10.0] },
            Wire { id: "w2".to_string(), net: Some("VCC".to_string()), from: [44.76, 37.46], to: [44.76, 10.0] },
            Wire { id: "w3".to_string(), net: Some("VCC".to_string()), from: [79.76, 18.0], to: [79.76, 10.0] },
            Wire { id: "w4".to_string(), net: Some("GND".to_string()), from: [10.0, 92.0], to: [150.0, 92.0] },
            Wire { id: "w5".to_string(), net: Some("GND".to_string()), from: [44.76, 40.0], to: [44.76, 92.0] },
            Wire { id: "w6".to_string(), net: Some("PA0".to_string()), from: [75.24, 37.46], to: [95.0, 37.46] },
            Wire { id: "w7".to_string(), net: Some("PA0".to_string()), from: [95.0, 37.46], to: [95.0, 46.76] },
            Wire { id: "w8".to_string(), net: Some("PA1".to_string()), from: [75.24, 40.0], to: [114.76, 40.0] },
            Wire { id: "w9".to_string(), net: Some("PA1".to_string()), from: [114.76, 40.0], to: [114.76, 62.0] },
            Wire { id: "w10".to_string(), net: Some("PA1".to_string()), from: [114.76, 62.0], to: [122.24, 62.0] },
            Wire { id: "w11".to_string(), net: Some("PB0".to_string()), from: [75.24, 42.54], to: [85.0, 42.54] },
            Wire { id: "w12".to_string(), net: Some("LED".to_string()), from: [137.76, 18.0], to: [150.0, 18.0] },
        ],
        junctions: vec![Junction { x: 44.76, y: 10.0 }, Junction { x: 10.0, y: 25.0 }],
        no_connects: vec![NoConnect { x: 75.24, y: 42.54, node: None }],
        texts: vec![TextLabel { text: "POWER SECTION 24V".to_string(), x: 12.0, y: 100.0 }],
        labels: vec![
            NetLabel {
                id: "l1".to_string(),
                text: "VCC".to_string(),
                x: 70.0,
                y: 7.0,
                scope: LabelScope::Global,
                rotation: 0.0,
                justify: "left".to_string(),
            },
            NetLabel {
                id: "l2".to_string(),
                text: "PA0".to_string(),
                x: 96.0,
                y: 37.46,
                scope: LabelScope::Local,
                rotation: 0.0,
                justify: "left".to_string(),
            },
            NetLabel {
                id: "l3".to_string(),
                text: "DATA0".to_string(),
                x: 116.0,
                y: 38.0,
                scope: LabelScope::Hierarchical,
                rotation: 0.0,
                justify: "left".to_string(),
            },
        ],
    };
    let sheet_two = Sheet {
        id: "sheet-2".to_string(),
        name: "Signal".to_string(),
        components: vec![
            component("U2", "ADC", 70.0, 45.0, &["VCC", "GND", "IN0", "IN1", "IN2", "IN3", "SCL", "SDA"]),
            component("R3", "4k7", 30.0, 20.0, &["1", "2"]),
            component("R4", "1k", 30.0, 65.0, &["1", "2"]),
            component("R5", "100k", 145.0, 70.0, &["1", "2"]),
            component("C3", "22p", 110.0, 22.0, &["1", "2"]),
            component("C4", "1u", 110.0, 78.0, &["1", "2"]),
            component("Q1", "NPN", 145.0, 25.0, &["B", "C", "E"]),
            component("J2", "CONN", 150.0, 50.0, &["1", "2"]),
        ],
        nets: vec![Net { id: "net-2".to_string(), name: "IN2".to_string(), nodes: vec![pin("U2", "IN2")] }],
        wires: vec![
            Wire { id: "s1".to_string(), net: Some("VCC".to_string()), from: [15.0, 12.0], to: [160.0, 12.0] },
            Wire { id: "s2".to_string(), net: Some("VCC".to_string()), from: [54.76, 41.27], to: [54.76, 12.0] },
            Wire { id: "s3".to_string(), net: Some("GND".to_string()), from: [15.0, 95.0], to: [160.0, 95.0] },
            Wire { id: "s4".to_string(), net: Some("GND".to_string()), from: [54.76, 43.73], to: [54.76, 95.0] },
            Wire { id: "s5".to_string(), net: Some("IN0".to_string()), from: [85.24, 41.27], to: [102.0, 41.27] },
            Wire { id: "s6".to_string(), net: Some("IN0".to_string()), from: [102.0, 41.27], to: [102.0, 22.0] },
            Wire { id: "s7".to_string(), net: Some("IN1".to_string()), from: [85.24, 43.73], to: [95.0, 43.73] },
            Wire { id: "s8".to_string(), net: Some("IN2".to_string()), from: [45.24, 65.0], to: [54.76, 65.0] },
            Wire { id: "s9".to_string(), net: Some("IN2".to_string()), from: [54.76, 65.0], to: [54.76, 46.19] },
            Wire { id: "s10".to_string(), net: Some("SCL".to_string()), from: [129.76, 25.0], to: [132.46, 25.0] },
            Wire { id: "s11".to_string(), net: Some("SCL".to_string()), from: [157.46, 25.0], to: [160.0, 25.0] },
        ],
        junctions: vec![Junction { x: 54.76, y: 12.0 }, Junction { x: 102.0, y: 41.27 }],
        no_connects: vec![NoConnect { x: 85.24, y: 46.19, node: None }],
        texts: vec![TextLabel { text: "SIGNAL PATH".to_string(), x: 15.0, y: 102.0 }],
        labels: vec![
            NetLabel {
                id: "s-l1".to_string(),
                text: "VCC".to_string(),
                x: 40.0,
                y: 9.0,
                scope: LabelScope::Global,
                rotation: 0.0,
                justify: "left".to_string(),
            },
            NetLabel {
                id: "s-l2".to_string(),
                text: "IN0".to_string(),
                x: 96.0,
                y: 41.27,
                scope: LabelScope::Local,
                rotation: 0.0,
                justify: "left".to_string(),
            },
            NetLabel {
                id: "s-l3".to_string(),
                text: "DATA1".to_string(),
                x: 118.0,
                y: 60.0,
                scope: LabelScope::Hierarchical,
                rotation: 0.0,
                justify: "left".to_string(),
            },
        ],
    };
    Design {
        id: "crowded".to_string(),
        title: Some("Crowded".to_string()),
        rev: "C".to_string(),
        company: "OpenLab".to_string(),
        date: "2026-11-25".to_string(),
        revision: 1,
        units: "mm".to_string(),
        sheets: vec![sheet_one, sheet_two],
        buses: Vec::new(),
        variants: Vec::new(),
        hierarchy: Vec::new(),
        rules: DesignRules::default(),
        next_component: 17,
        next_net: 3,
        next_wire: 13,
    }
}

/// Every text of the base schematic, including rotated pin names, stays inside
/// the viewport and clear of every other text.
#[test]
fn schematic_svg_keeps_net_pin_and_component_labels_apart() {
    let design = crowded_design();
    let sheets: Vec<&Sheet> = design.sheets.iter().collect();
    let svg = crate::render_svg_for(&design, &sheets);
    let frame = view_box(&svg);
    let frame = [frame[0], frame[1], frame[0] + frame[2], frame[1] + frame[3]];
    let texts = placed_texts(&svg);

    assert!(texts.len() > 40, "the schematic paints every label: {}", texts.len());
    assert_eq!(texts.iter().filter(|run| run.text == "PB0").count(), 2, "the net name and the pin name are both painted");
    assert!(texts.iter().any(|run| run.text == "4u7"), "a rotated inductor keeps its value");
    assert!(texts.iter().any(|run| run.text == "2k2"), "the crowded passive keeps its value");

    assert_texts_inside(&texts, frame, "schematic svg");
    assert_texts_apart(&texts, "schematic svg");
    assert_eq!(svg, crate::render_svg_for(&design, &sheets), "the label layout is deterministic");
}

/// One sheet and several sheets share the same layout rules.
#[test]
fn single_page_schematic_svg_keeps_every_text_apart() {
    let design = crowded_design();
    let sheets = [&design.sheets[0]];
    let svg = crate::render_svg_for(&design, &sheets);
    let frame = view_box(&svg);
    let frame = [frame[0], frame[1], frame[0] + frame[2], frame[1] + frame[3]];
    let texts = placed_texts(&svg);
    assert_texts_inside(&texts, frame, "one-page schematic");
    assert_texts_apart(&texts, "one-page schematic");
}

/// The thumbnail inherits the same planned component labels and drops the
/// fixed-position sheet texts that would collide at cell scale.
#[test]
fn sheet_overview_scales_labels_without_overlap_or_page_overflow() {
    let design = crowded_design();
    let svg = render_sheet_overview(&design).svg;
    let page = view_box(&svg);
    let frame = [page[0], page[1], page[0] + page[2], page[1] + page[3]];
    let texts = placed_texts(&svg);

    assert!(!texts.iter().any(|run| run.text == "POWER SECTION 24V"), "a fixed block title is omitted in the thumbnail");
    assert!(!texts.iter().any(|run| run.text == "SIGNAL PATH"), "the second block title is omitted too");
    assert!(texts.iter().any(|run| run.text == "U1") && texts.iter().any(|run| run.text == "U2"));
    assert!(texts.len() > 40, "the thumbnail still paints the component and net labels");
    assert_texts_inside(&texts, frame, "sheet overview");
    assert_texts_apart(&texts, "sheet overview");
    assert_eq!(svg, render_sheet_overview(&design).svg, "the thumbnail layout is deterministic");
}

/// Every SVG drawing plus the base schematic export, labelled for assertions.
fn all_svg_exports(design: &Design) -> Vec<(&'static str, String)> {
    // The schematic is exercised as a one-sheet page here; the multi-sheet
    // pagination has its own dedicated assertions.
    let sheets: Vec<&Sheet> = design.sheets.iter().take(1).collect();
    vec![
        ("block diagram", render_block_diagram(design).svg),
        ("sheet overview", render_sheet_overview(design).svg),
        ("pinout chart", render_pinout_chart(design).svg),
        ("symbol library", render_symbol_library(design).svg),
        ("net topology", render_net_topology(design).svg),
        ("power distribution", render_power_distribution(design).svg),
        ("bom", render_bom_drawing(design).svg),
        ("net xref", render_net_xref(design).svg),
        ("hierarchy", render_hierarchy(design).svg),
        ("erc", render_erc(design).svg),
        ("schematic svg", crate::render_svg_for(design, &sheets)),
    ]
}

/// Every text tag that carries `needle` anywhere in its attributes.
fn tags_with(fragment: &str, needle: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut rest = fragment;
    while let Some(at) = rest.find(needle) {
        let start = rest[..at].rfind('<').expect("tag start");
        let end = start + rest[start..].find('>').expect("tag end");
        tags.push(rest[start..end].to_string());
        rest = &rest[end..];
    }
    tags
}

#[test]
fn all_exports_use_a4_with_an_inner_frame_and_keep_content_inside_it() {
    let design = sample_design();
    for (name, svg) in all_svg_exports(&design) {
        assert_eq!(view_box(&svg), [0.0, 0.0, 297.0, 210.0], "{name} is A4 landscape");
        assert_eq!(occurrences(&svg, r#"data-title-block="true""#), 1, "{name} has one title block");
        assert_eq!(occurrences(&svg, r#"data-drawing=""#), 1, "{name} has one fitted content group");
        let frames = rects_with(&svg, r#"data-page-frame="true""#);
        assert_eq!(frames.len(), 1, "{name} paints exactly one inner frame");
        let frame = frames[0];
        assert!((frame[0] - 23.76).abs() < 0.1 && (frame[1] - 16.80).abs() < 0.1, "{name} inner frame origin: {frame:?}");
        assert!((frame[2] - 273.24).abs() < 0.1 && (frame[3] - 171.20).abs() < 0.1, "{name} inner frame size: {frame:?}");

        let page = page(&svg);
        let content = [
            page.translate.0,
            page.translate.1,
            page.translate.0 + page.content_size.0 * page.scale,
            page.translate.1 + page.content_size.1 * page.scale,
        ];
        assert!(
            content[0] >= frame[0] - 1.0
                && content[1] >= frame[1] - 1.0
                && content[2] <= frame[2] + 1.0
                && content[3] <= frame[3] + 1.0,
            "{name} fitted content leaves the inner frame: {content:?} vs {frame:?}"
        );

        // The title block deliberately sits below the frame, so only compare
        // texts above it.
        let texts: Vec<PlacedText> = placed_texts(&svg).into_iter().filter(|run| run.bounds[3] <= frame[3] + 1.0).collect();
        for placed in &texts {
            let bounds = placed.bounds;
            assert!(
                bounds[0] >= frame[0] - 1.0
                    && bounds[1] >= frame[1] - 1.0
                    && bounds[2] <= frame[2] + 1.0
                    && bounds[3] <= frame[3] + 1.0,
                "{name} paints `{}` outside the inner frame {frame:?} at {bounds:?}",
                placed.text
            );
        }
    }
}

#[test]
fn no_svg_export_truncates_text_with_an_ellipsis() {
    let design = sample_design();
    for (name, svg) in all_svg_exports(&design) {
        assert!(!svg.contains("..."), "{name} truncates text with an ellipsis");
    }
}

#[test]
fn table_text_stays_inside_its_declared_cell() {
    let mut erc_design = sample_design();
    erc_design.sheets[0].components.push(component("U1", "DUPLICATE", 10.0, 10.0, &["1"]));
    let drawings = [
        ("bom", render_bom_drawing(&sample_design()).svg),
        ("net xref", render_net_xref(&sample_design()).svg),
        ("erc", render_erc(&erc_design).svg),
    ];
    for (name, svg) in &drawings {
        let page = page(svg);
        let mut checked = 0;
        for run in text_runs(&page.content) {
            let Some(left) = attribute(&run.attributes, " data-cell-left=\"")
            else {
                continue;
            };
            let width = attribute(&run.attributes, " data-cell-width=\"").expect("cell width");
            let bounds = text_box(&run, (0.0, 0.0), 1.0);
            assert!(
                bounds[0] >= left - 0.1 && bounds[2] <= left + width + 0.1,
                "{name} paints `{}` outside its cell [{left:.2}, {:.2}]: {bounds:?}",
                run.text,
                left + width
            );
            checked += 1;
        }
        assert!(checked > 0, "{name} paints table text with cell metadata");
    }
}

#[test]
fn erc_report_fills_the_page_without_a_sparse_position_column() {
    let mut design = sample_design();
    for index in 0..24 {
        design.sheets[0].components.push(component(&format!("X{index:02}"), "1k", 10.0 + index as f64, 10.0, &["1"]));
    }
    // Remove coordinates so the POSITION column would be mostly blank.
    for sheet in &mut design.sheets {
        sheet.wires.clear();
    }
    let drawing = render_erc(&design);
    let page = page(&drawing.svg);
    assert!(drawing.findings > 18, "enough findings to fill the table");
    assert!(!page.content.contains("POSITION"), "a sparse position column is dropped");
    assert!(
        page.scale > 0.95,
        "the report is not shrunk into unreadability (scale {:.3}, content {:?})",
        page.scale,
        page.content_size
    );
    let height = page.content_size.1 * page.scale;
    assert!(height >= drawable().1 * 0.9, "the report only covers {:.0}% of the page height", height / drawable().1 * 100.0);
    assert!(!page.content.contains("..."), "long descriptions wrap instead of truncating");
}

#[test]
fn net_topology_marks_every_unrelated_crossing_with_a_hop_and_real_junctions() {
    let design = sample_design();
    let page = page(&render_net_topology(&design).svg);

    let trunks: Vec<(f64, f64, f64)> = tags_with(&page.content, r#"data-trunk="true""#)
        .iter()
        .map(|tag| {
            (
                attribute(tag, " x1=\"").expect("trunk x"),
                attribute(tag, " y1=\"").expect("trunk top"),
                attribute(tag, " y2=\"").expect("trunk bottom"),
            )
        })
        .collect();
    assert!(!trunks.is_empty(), "the topology paints vertical trunks");

    let mut taps: BTreeMap<(String, i64), (f64, f64)> = BTreeMap::new();
    for tag in tags_with(&page.content, r#"data-net=""#) {
        let Some(net) = tag_text(&tag, "data-net=\"")
        else {
            continue;
        };
        if tag.starts_with("<line ") {
            let (Some(x1), Some(y1), Some(x2), Some(y2)) =
                (attribute(&tag, " x1=\""), attribute(&tag, " y1=\""), attribute(&tag, " x2=\""), attribute(&tag, " y2=\""))
            else {
                continue;
            };
            if (y1 - y2).abs() > 0.01 {
                continue;
            }
            let entry = taps.entry((net.to_string(), (y1 * 100.0).round() as i64)).or_insert((x1.min(x2), x1.max(x2)));
            entry.0 = entry.0.min(x1).min(x2);
            entry.1 = entry.1.max(x1).max(x2);
        }
        else if tag.starts_with("<path ") {
            let at = tag_text(&tag, "data-hop-at=\"").expect("hop point");
            let mut numbers = at.split_whitespace().map(|number| number.parse::<f64>().expect("hop number"));
            let x = numbers.next().expect("hop x");
            let y = numbers.next().expect("hop y");
            let entry = taps.entry((net.to_string(), (y * 100.0).round() as i64)).or_insert((x - 0.9, x + 0.9));
            entry.0 = entry.0.min(x - 0.9);
            entry.1 = entry.1.max(x + 0.9);
        }
    }

    let mut junctions: BTreeMap<(String, i64), f64> = BTreeMap::new();
    for tag in tags_with(&page.content, r#"data-junction="true""#) {
        let net = tag_text(&tag, "data-net=\"").expect("junction net");
        let x = attribute(&tag, " cx=\"").expect("junction x");
        let y = attribute(&tag, " cy=\"").expect("junction y");
        junctions.insert((net.to_string(), (y * 100.0).round() as i64), x);
    }

    let mut hops: Vec<(f64, f64)> = Vec::new();
    for tag in tags_with(&page.content, "data-hop-at=\"") {
        let at = tag_text(&tag, "data-hop-at=\"").expect("hop");
        let mut numbers = at.split_whitespace().map(|number| number.parse::<f64>().expect("hop number"));
        hops.push((numbers.next().expect("hop x"), numbers.next().expect("hop y")));
    }

    let mut crossings = 0;
    for ((net, y_key), (min_x, max_x)) in &taps {
        let y = *y_key as f64 / 100.0;
        let destination = junctions
            .get(&(net.clone(), *y_key))
            .unwrap_or_else(|| panic!("net {net} at y={y:.2} has no junction at its trunk"));
        assert!(
            trunks.iter().any(|(x, top, bottom)| (x - destination).abs() < 0.05 && *top <= y && y <= *bottom),
            "junction for {net} at y={y:.2} is not on a trunk"
        );
        for (x, top, bottom) in &trunks {
            if (x - destination).abs() < 0.05 || *x <= min_x + 0.05 || *x >= max_x - 0.05 || y < *top || y > *bottom {
                continue;
            }
            crossings += 1;
            assert!(
                hops.iter().any(|(hop_x, hop_y)| (*hop_x - x).abs() < 0.05 && (*hop_y - y).abs() < 0.05),
                "net {net} crosses trunk x={x:.2} at y={y:.2} without a hop"
            );
        }
    }
    assert!(crossings > 0, "the sample topology has crossings");
    assert!(!hops.is_empty(), "crossings paint hop arcs");
}

#[test]
fn symbol_library_keeps_every_pin_name_without_truncation() {
    let design = sample_design();
    let page = page(&render_symbol_library(&design).svg);
    assert!(!page.content.contains("..."), "pin lists are wrapped, never elided");
    for pin in ["IO1", "IO2", "VCC", "GND"] {
        assert!(page.content.contains(pin), "the complete pin list still contains {pin}");
    }
    assert_eq!(occurrences(&page.content, "PINS:"), 6, "one complete pin list per distinct symbol");
    let cells = group_rects(&page.content, "data-symbol=\"");
    assert_eq!(cells.len(), 6);
    assert_no_overlap(&cells, "symbol cards");
    assert!(
        page.content_size.0 * page.scale >= drawable().0 * 0.95 && page.content_size.1 * page.scale >= drawable().1 * 0.95,
        "the symbol cards fill the drawable area"
    );
}

#[test]
fn schematic_svg_paginates_one_full_a4_page_per_sheet() {
    let design = crowded_design();
    let sheets: Vec<&Sheet> = design.sheets.iter().collect();
    let sheet_count = sheets.len();
    let svg = crate::render_svg_for(&design, &sheets);
    assert!(!svg.contains("..."), "the schematic never truncates text");
    assert_eq!(occurrences(&svg, r#"data-page=""#), sheet_count, "one SVG page per sheet");
    assert_eq!(occurrences(&svg, r#"data-drawing=""#), sheet_count, "each page has its own fitted content");
    assert_eq!(occurrences(&svg, r#"data-title-block="true""#), sheet_count, "each page has its own title block");
    let page_frames = rects_with(&svg, r#"data-page-frame="true""#);
    assert_eq!(page_frames.len(), sheet_count, "each page has its own inner frame");

    let [view_x, view_y, view_width, view_height] = view_box(&svg);
    assert_eq!([view_x, view_y, view_width], [0.0, 0.0, 297.0]);
    let gap = (view_height - 210.0 * sheet_count as f64) / (sheet_count.saturating_sub(1)) as f64;
    assert!((6.0..=10.0).contains(&gap), "page gap is {gap:.2} mm");
    assert!(view_height >= 210.0 * sheet_count as f64, "pages do not overlap");

    let frames: Vec<[f64; 4]> = {
        let mut frames = page_frames;
        frames.sort_by(|left, right| left[1].total_cmp(&right[1]));
        frames
    };
    let contents = tags_with(&svg, r#"data-drawing=""#);
    assert_eq!(contents.len(), sheet_count);
    let mut scales = Vec::new();
    for (index, (frame, tag)) in frames.iter().zip(&contents).enumerate() {
        let page_top = index as f64 * (210.0 + gap);
        assert!((frame[0] - 23.76).abs() < 0.1, "page {index} frame x");
        assert!((frame[1] - (page_top + 16.80)).abs() < 0.1, "page {index} frame y: {frame:?}");
        assert!((frame[2] - 273.24).abs() < 0.1, "page {index} frame right");
        assert!((frame[3] - (page_top + 171.20)).abs() < 0.1, "page {index} frame bottom");

        let sizes: Vec<f64> = tag_text(tag, "data-content=\"")
            .expect("content size")
            .split_whitespace()
            .map(|number| number.parse().expect("content number"))
            .collect();
        let scale: f64 = tag_text(tag, "data-scale=\"").expect("content scale").parse().expect("scale number");
        let transform = parse_transform(tag_text(tag, " transform=\"").expect("content transform"));
        scales.push(scale);
        let used_width = sizes[0] * scale;
        let used_height = sizes[1] * scale;
        assert!(
            used_width >= (frame[2] - frame[0]) * 0.9 || used_height >= (frame[3] - frame[1]) * 0.9,
            "page {index} content only fills {used_width:.1}x{used_height:.1} of a {:.1}x{:.1} frame",
            frame[2] - frame[0],
            frame[3] - frame[1]
        );
        assert!(
            transform.e >= frame[0] - 1.0
                && transform.f >= frame[1] - 1.0
                && transform.e + used_width <= frame[2] + 1.0
                && transform.f + used_height <= frame[3] + 1.0,
            "page {index} content leaves its inner frame: {frame:?}"
        );
    }

    let all_texts = placed_texts(&svg);
    for (index, frame) in frames.iter().enumerate() {
        let page_top = index as f64 * (210.0 + gap);
        let texts: Vec<&PlacedText> =
            all_texts.iter().filter(|run| run.bounds[1] >= page_top - 1.0 && run.bounds[3] <= page_top + 188.0 + 1.0).collect();
        assert!(!texts.is_empty(), "page {index} paints text");
        for run in &texts {
            let bounds = run.bounds;
            assert!(
                bounds[0] >= frame[0] - 1.0
                    && bounds[1] >= frame[1] - 1.0
                    && bounds[2] <= frame[2] + 1.0
                    && bounds[3] <= frame[3] + 1.0,
                "page {index} paints `{}` outside its frame {frame:?} at {bounds:?}",
                run.text
            );
        }
    }
    let min_text = all_texts.iter().map(|run| run.size).fold(f64::INFINITY, f64::min);
    assert!(min_text >= 2.0, "smallest glyph is only {min_text:.2} mm tall (scales {scales:?})");
    assert_texts_apart(&all_texts, "schematic pages");
}

/// Every `/F1 <size> Tf` font size in a PDF content stream, in points.
fn pdf_font_sizes(pdf: &str) -> Vec<f64> {
    let mut sizes = Vec::new();
    let mut rest = pdf;
    while let Some(at) = rest.find("/F1 ") {
        let after = &rest[at + 4..];
        let Some(end) = after.find(" Tf")
        else {
            break;
        };
        if let Ok(size) = after[..end].trim().parse::<f64>() {
            sizes.push(size);
        }
        rest = &after[end..];
    }
    sizes
}

#[test]
fn schematic_pdf_uses_one_a4_page_per_sheet_with_at_least_two_millimetre_text() {
    let design = crowded_design();
    let sheets: Vec<&Sheet> = design.sheets.iter().collect();
    let (pdf, _) = crate::render_pdf_for(&design, &sheets);
    assert_eq!(occurrences(&pdf, "/Type /Page /Parent"), sheets.len(), "one PDF page per sheet");
    assert_eq!(occurrences(&pdf, "/MediaBox [0 0 842 595]"), sheets.len(), "every PDF page is A4 landscape");
    let sizes = pdf_font_sizes(&pdf);
    assert!(!sizes.is_empty(), "the PDF paints text");
    let smallest = sizes.iter().copied().fold(f64::INFINITY, f64::min);
    let millimetres = smallest * 25.4 / 72.0;
    assert!(millimetres >= 2.0, "smallest PDF glyph is {millimetres:.2} mm ({smallest:.2} pt)");
    assert!(!pdf.contains("..."), "the schematic PDF never truncates text");
}

#[test]
fn pdf_exports_never_elide_net_xref_or_bom_content() {
    let design = sample_design();
    let (xref, _, pages, nets) = crate::render_net_xref_pdf(&design);
    assert!(pages >= 1 && nets > 0);
    assert!(!xref.contains("..."), "net xref PDF still contains an ellipsis");
    for row in crate::drawings::net_xref_rows(&design) {
        for node in row.nodes {
            assert!(xref.contains(&node), "net xref PDF dropped node `{node}`");
        }
    }
    let (bom, _, _) = crate::render_bom_pdf(&design);
    assert!(!bom.contains("..."), "BOM PDF still contains an ellipsis");
}
