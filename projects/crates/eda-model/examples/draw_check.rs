//! Temporary visual audit driver: builds a two-sheet design and writes every
//! drawing projection into one directory for screenshot review.

use std::{fs, path::PathBuf};

use open_eda_model::{
    AddBusRequest, AddComponentRequest, AddHierarchyPortRequest, AddHierarchyRequest, AddJunctionRequest, AddLabelRequest,
    AddNoConnectRequest, AddTextRequest, AddWireRequest, ConnectNetRequest, CreateRequest, LabelScope, PortDirection, add_bus,
    add_component, add_hierarchy, add_hierarchy_port, add_junction, add_label, add_no_connect, add_sheet, add_text, add_wire,
    connect_net, create, derive_nets, export,
};

#[allow(clippy::too_many_arguments)]
fn part(
    handle: u64,
    sheet: Option<&str>,
    reference: &str,
    value: &str,
    x: f64,
    y: f64,
    rotation: f64,
    pins: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    add_component(
        handle,
        AddComponentRequest {
            sheet: sheet.map(str::to_string),
            reference: reference.into(),
            value: value.into(),
            x,
            y,
            rotation,
            pins: pins.iter().map(|pin| (*pin).to_string()).collect(),
        },
    )?;
    Ok(())
}

fn wire(
    handle: u64,
    sheet: Option<&str>,
    from: [f64; 2],
    to: [f64; 2],
    net: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    add_wire(handle, AddWireRequest { sheet: sheet.map(str::to_string), net: net.map(str::to_string), from, to })?;
    Ok(())
}

fn label(
    handle: u64,
    sheet: Option<&str>,
    text: &str,
    x: f64,
    y: f64,
    scope: LabelScope,
) -> Result<(), Box<dyn std::error::Error>> {
    add_label(
        handle,
        AddLabelRequest {
            sheet: sheet.map(str::to_string),
            text: text.into(),
            x,
            y,
            scope: Some(scope),
            rotation: None,
            justify: None,
        },
    )?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out: PathBuf = match std::env::args().nth(1) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(".draw-check"),
    };
    fs::create_dir_all(&out)?;

    let created = create(CreateRequest {
        title: Some("Sensor Node 传感节点".into()),
        rev: Some("C".into()),
        company: Some("OpenLab".into()),
        date: Some("2026-11-25".into()),
        template: Some("blank".into()),
    })?;
    let handle = created["handle"].as_u64().expect("handle");

    // ---- Sheet 1 "Power": regulator, bulk caps, LED, inductor. ----
    part(handle, None, "U1", "MCU", 60.0, 40.0, 0.0, &["VCC", "GND", "PA0", "PA1", "PB0", "PB1"])?;
    part(handle, None, "R1", "10k", 95.0, 18.0, 0.0, &["1", "2"])?;
    part(handle, None, "R2", "330", 95.0, 62.0, 90.0, &["1", "2"])?;
    part(handle, None, "C1", "100n", 25.0, 40.0, 0.0, &["1", "2"])?;
    part(handle, None, "C2", "10u+", 25.0, 72.0, 90.0, &["1", "2"])?;
    part(handle, None, "D1", "LED", 130.0, 18.0, 0.0, &["A", "K"])?;
    part(handle, None, "L1", "4u7", 130.0, 62.0, 90.0, &["1", "2"])?;

    wire(handle, None, [10.0, 10.0], [150.0, 10.0], Some("VCC"))?;
    wire(handle, None, [44.76, 37.46], [44.76, 10.0], Some("VCC"))?;
    wire(handle, None, [79.76, 18.0], [79.76, 10.0], Some("VCC"))?;
    wire(handle, None, [10.0, 92.0], [150.0, 92.0], Some("GND"))?;
    wire(handle, None, [44.76, 40.0], [44.76, 92.0], Some("GND"))?;
    wire(handle, None, [37.46, 25.0], [10.0, 25.0], Some("GND"))?;
    wire(handle, None, [10.0, 25.0], [10.0, 92.0], Some("GND"))?;
    wire(handle, None, [75.24, 37.46], [95.0, 37.46], Some("PA0"))?;
    wire(handle, None, [95.0, 37.46], [95.0, 46.76], Some("PA0"))?;
    wire(handle, None, [75.24, 40.0], [114.76, 40.0], Some("PA1"))?;
    wire(handle, None, [114.76, 40.0], [114.76, 62.0], Some("PA1"))?;
    wire(handle, None, [114.76, 62.0], [122.24, 62.0], Some("PA1"))?;
    wire(handle, None, [75.24, 42.54], [85.0, 42.54], Some("PB0"))?;
    wire(handle, None, [137.76, 18.0], [150.0, 18.0], Some("LED"))?;

    add_junction(handle, AddJunctionRequest { sheet: None, x: 44.76, y: 10.0 })?;
    add_junction(handle, AddJunctionRequest { sheet: None, x: 10.0, y: 25.0 })?;
    add_no_connect(handle, AddNoConnectRequest { sheet: None, x: 75.24, y: 42.54, node: None })?;

    label(handle, None, "VCC", 70.0, 7.0, LabelScope::Global)?;
    label(handle, None, "PA0", 96.0, 37.46, LabelScope::Local)?;
    label(handle, None, "DATA0", 116.0, 38.0, LabelScope::Hierarchical)?;
    add_text(handle, AddTextRequest { sheet: None, text: "POWER SECTION 24V".into(), x: 12.0, y: 100.0 })?;

    // ---- Sheet 2 "Signal": ADC, passives, a transistor and a jack. ----
    let second = add_sheet(handle, Some("Signal"))?;
    let sheet2 = second["sheet"]["id"].as_str().expect("sheet id").to_string();
    let s2 = Some(sheet2.as_str());
    part(handle, s2, "U2", "ADC", 70.0, 45.0, 0.0, &["VCC", "GND", "IN0", "IN1", "IN2", "IN3", "SCL", "SDA"])?;
    part(handle, s2, "R3", "4k7", 30.0, 20.0, 45.0, &["1", "2"])?;
    part(handle, s2, "R4", "1k", 30.0, 65.0, 0.0, &["1", "2"])?;
    part(handle, s2, "R5", "100k", 145.0, 70.0, 0.0, &["1", "2"])?;
    part(handle, s2, "C3", "22p", 110.0, 22.0, 0.0, &["1", "2"])?;
    part(handle, s2, "C4", "1u", 110.0, 78.0, 90.0, &["1", "2"])?;
    part(handle, s2, "Q1", "NPN", 145.0, 25.0, 0.0, &["B", "C", "E"])?;
    part(handle, s2, "J2", "CONN", 150.0, 50.0, 90.0, &["1", "2"])?;

    wire(handle, s2, [15.0, 12.0], [160.0, 12.0], Some("VCC"))?;
    wire(handle, s2, [54.76, 41.27], [54.76, 12.0], Some("VCC"))?;
    wire(handle, s2, [15.0, 95.0], [160.0, 95.0], Some("GND"))?;
    wire(handle, s2, [54.76, 43.73], [54.76, 95.0], Some("GND"))?;
    wire(handle, s2, [85.24, 41.27], [102.0, 41.27], Some("IN0"))?;
    wire(handle, s2, [102.0, 41.27], [102.0, 22.0], Some("IN0"))?;
    wire(handle, s2, [102.0, 22.0], [102.24, 22.0], Some("IN0"))?;
    wire(handle, s2, [85.24, 43.73], [95.0, 43.73], Some("IN1"))?;
    wire(handle, s2, [45.24, 65.0], [54.76, 65.0], Some("IN2"))?;
    wire(handle, s2, [54.76, 65.0], [54.76, 46.19], Some("IN2"))?;
    wire(handle, s2, [129.76, 25.0], [132.46, 25.0], Some("SCL"))?;
    wire(handle, s2, [157.46, 25.0], [160.0, 25.0], Some("SCL"))?;

    add_junction(handle, AddJunctionRequest { sheet: Some(sheet2.clone()), x: 54.76, y: 12.0 })?;
    add_junction(handle, AddJunctionRequest { sheet: Some(sheet2.clone()), x: 102.0, y: 41.27 })?;
    add_no_connect(handle, AddNoConnectRequest { sheet: Some(sheet2.clone()), x: 85.24, y: 46.19, node: None })?;

    label(handle, s2, "VCC", 40.0, 9.0, LabelScope::Global)?;
    label(handle, s2, "IN0", 96.0, 41.27, LabelScope::Local)?;
    label(handle, s2, "DATA1", 118.0, 60.0, LabelScope::Hierarchical)?;
    add_text(handle, AddTextRequest { sheet: Some(sheet2.clone()), text: "SIGNAL PATH".into(), x: 15.0, y: 102.0 })?;

    add_bus(handle, AddBusRequest { name: "DATA".into(), width: 4, aliases: vec!["BUS".into()] })?;

    // Hierarchical sheet symbols for the hierarchy drawing.
    for (name, file, x) in [("Power Page", "power.kicad_sch", 20.0), ("Signal Page", "signal.kicad_sch", 100.0)] {
        add_hierarchy(
            handle,
            AddHierarchyRequest { name: name.into(), file: file.into(), x, y: 140.0, width: 60.0, height: 40.0 },
        )?;
    }
    for (sheet, port, direction) in [
        ("Power Page", "VCC", PortDirection::Input),
        ("Power Page", "GND", PortDirection::Passive),
        ("Signal Page", "IN0", PortDirection::Input),
        ("Signal Page", "SCL", PortDirection::Bidirectional),
    ] {
        add_hierarchy_port(
            handle,
            AddHierarchyPortRequest { sheet: sheet.into(), name: port.into(), direction, x: 0.0, y: 0.0 },
        )?;
    }

    // Attach connectivity so the table drawings carry real net membership.
    derive_nets(handle, None)?;
    derive_nets(handle, Some(&sheet2))?;
    for (sheet, net, nodes) in [
        (None, "VCC", &["U1.VCC", "R1.2", "C1.2"][..]),
        (None, "GND", &["U1.GND", "C1.1", "C2.2"][..]),
        (None, "PA0", &["U1.PA0", "R1.1"][..]),
        (None, "PA1", &["U1.PA1", "R2.1"][..]),
        (None, "PB0", &["U1.PB0"][..]),
        (None, "LED", &["R2.2", "D1.A"][..]),
        (Some(sheet2.as_str()), "VCC", &["U2.VCC", "R3.1", "C3.2"][..]),
        (Some(sheet2.as_str()), "GND", &["U2.GND", "R4.1", "C4.1", "R5.1"][..]),
        (Some(sheet2.as_str()), "IN0", &["U2.IN0", "C3.1"][..]),
        (Some(sheet2.as_str()), "IN1", &["U2.IN1"][..]),
        (Some(sheet2.as_str()), "IN2", &["U2.IN2", "R4.2"][..]),
        (Some(sheet2.as_str()), "SCL", &["Q1.B", "J2.1"][..]),
    ] {
        connect_net(
            handle,
            ConnectNetRequest {
                sheet: sheet.map(str::to_string),
                net: net.into(),
                nodes: nodes.iter().map(|node| (*node).to_string()).collect(),
            },
        )?;
    }

    for format in [
        "svg",
        "pdf",
        "png",
        "block_diagram",
        "sheet_overview",
        "pinout_chart",
        "symbol_library",
        "net_topology",
        "power_distribution",
        "bom_drawing",
        "bom_drawing_pdf",
        "net_xref",
        "net_xref_pdf",
        "hierarchy",
        "erc",
    ] {
        let extension = if format == "png" {
            "png"
        }
        else if format == "pdf" || format.ends_with("_pdf") {
            "pdf"
        }
        else {
            "svg"
        };
        let target = out.join(format!("drawing_{format}.{extension}"));
        match export(handle, format, &target) {
            Ok(report) => println!("OK {format}: {report}"),
            Err(err) => println!("ERR {format}: {}", err.message),
        }
    }
    Ok(())
}
