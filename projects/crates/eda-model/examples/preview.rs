//! Temporary preview driver: builds a demo schematic and exports SVG and PDF.

use std::path::Path;

use open_eda_model::{
    AddComponentRequest, AddJunctionRequest, AddLabelRequest, AddNoConnectRequest, AddTextRequest, AddWireRequest,
    CreateRequest, LabelScope, add_component, add_junction, add_label, add_no_connect, add_text, add_wire, create, export,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args().nth(1).unwrap_or_else(|| "preview".into());
    let out = Path::new(&out);
    std::fs::create_dir_all(out)?;

    let created = create(CreateRequest {
        title: Some("OpenEDA demo sheet".into()),
        rev: None,
        company: None,
        date: None,
        template: Some("blank".into()),
    })?;
    let handle = created["handle"].as_u64().expect("handle");

    // The circuit: C1 decouples VCC from GND at U1, R1 pulls IO1 up to VCC,
    // and IO2 drives D1 through R2 into GND.
    // Pin stubs are 2.54 long, so wire endpoints below land exactly on the
    // rendered connection points instead of stopping at the body outline.
    let parts = [
        ("U1", "MCU", 60.0, 40.0, vec!["VCC".to_string(), "GND".to_string(), "IO1".to_string(), "IO2".to_string()]),
        ("R1", "10k", 95.0, 25.0, vec!["1".to_string(), "2".to_string()]),
        ("R2", "330", 95.0, 55.0, vec!["1".to_string(), "2".to_string()]),
        ("C1", "100n", 20.0, 38.73, vec!["1".to_string(), "2".to_string()]),
        ("D1", "LED", 130.0, 55.0, vec!["A".to_string(), "K".to_string()]),
        ("J1", "CONN", 145.0, 25.0, vec!["1".to_string(), "2".to_string()]),
    ];
    for (reference, value, x, y, pins) in parts {
        add_component(
            handle,
            AddComponentRequest { sheet: None, reference: reference.into(), value: value.into(), x, y, rotation: 0.0, pins },
        )?;
    }

    let wires = [
        // VCC: C1 pin 2 to U1 VCC, R1 pin 2 along the top rail down to U1.
        ([35.24, 38.73], [44.76, 38.73], Some("VCC")),
        ([110.24, 25.0], [110.24, 16.0], Some("VCC")),
        ([110.24, 16.0], [44.76, 16.0], Some("VCC")),
        ([44.76, 16.0], [44.76, 38.73], Some("VCC")),
        // GND: C1 pin 1 along the bottom rail to U1 GND and to D1 K.
        ([4.76, 38.73], [4.76, 64.0], Some("GND")),
        ([4.76, 64.0], [145.24, 64.0], Some("GND")),
        ([44.76, 64.0], [44.76, 41.27], Some("GND")),
        ([145.24, 55.0], [145.24, 64.0], Some("GND")),
        // IO1: U1 to the R1 pull-up.
        ([75.24, 38.73], [77.5, 38.73], Some("IO1")),
        ([77.5, 38.73], [77.5, 25.0], Some("IO1")),
        ([77.5, 25.0], [79.76, 25.0], Some("IO1")),
        // IO2: U1 to R2.
        ([75.24, 41.27], [77.5, 41.27], Some("IO2")),
        ([77.5, 41.27], [77.5, 55.0], Some("IO2")),
        ([77.5, 55.0], [79.76, 55.0], Some("IO2")),
        // LED: R2 pin 2 to D1 A.
        ([110.24, 55.0], [114.76, 55.0], Some("LED")),
    ];
    for (from, to, net) in wires {
        add_wire(handle, AddWireRequest { sheet: None, net: net.map(str::to_string), from, to })?;
    }
    add_text(handle, AddTextRequest { sheet: None, text: "Demo schematic".into(), x: 4.76, y: 72.0 })?;

    // Connection dots where the GND risers meet the bottom rail.
    for x in [44.76, 145.24] {
        add_junction(handle, AddJunctionRequest { sheet: None, x, y: 64.0 })?;
    }
    // Net labels: a global power label, a local signal label and a hierarchy port.
    add_label(
        handle,
        AddLabelRequest {
            sheet: None,
            text: "VCC".into(),
            x: 60.0,
            y: 13.5,
            scope: Some(LabelScope::Global),
            rotation: None,
            justify: None,
        },
    )?;
    add_label(
        handle,
        AddLabelRequest {
            sheet: None,
            text: "IO1".into(),
            x: 78.5,
            y: 33.0,
            scope: Some(LabelScope::Local),
            rotation: None,
            justify: None,
        },
    )?;
    add_label(
        handle,
        AddLabelRequest {
            sheet: None,
            text: "LED".into(),
            x: 112.5,
            y: 50.0,
            scope: Some(LabelScope::Hierarchical),
            rotation: None,
            justify: None,
        },
    )?;
    // J1 pin 2 is deliberately left open.
    add_no_connect(
        handle,
        AddNoConnectRequest {
            sheet: None,
            x: 160.24,
            y: 25.0,
            node: Some(open_eda_model::Node { reference: "J1".into(), pin: "2".into() }),
        },
    )?;

    for format in ["svg", "pdf"] {
        let target = out.join(format!("demo-schematic.{format}"));
        let report = export(handle, format, &target)?;
        println!("{format}: {report}");
    }
    Ok(())
}
