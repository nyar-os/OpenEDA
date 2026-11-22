//! Temporary preview driver: builds a demo schematic and exports SVG and PDF.

use std::path::Path;

use open_eda_model::{
    AddComponentRequest, AddTextRequest, AddWireRequest, CreateRequest, add_component, add_text, add_wire, create, export,
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

    let parts = [
        ("U1", "MCU", 40.0, 40.0, vec!["VCC".to_string(), "GND".to_string(), "IO1".to_string(), "IO2".to_string()]),
        ("R1", "10k", 70.0, 24.0, vec!["1".to_string(), "2".to_string()]),
        ("R2", "330", 70.0, 56.0, vec!["1".to_string(), "2".to_string()]),
        ("C1", "100n", 20.0, 56.0, vec!["1".to_string(), "2".to_string()]),
    ];
    for (reference, value, x, y, pins) in parts {
        add_component(
            handle,
            AddComponentRequest { sheet: None, reference: reference.into(), value: value.into(), x, y, rotation: 0.0, pins },
        )?;
    }

    let wires = [
        ([20.0, 48.38], [20.0, 36.0], Some("VCC")),
        ([20.0, 36.0], [27.3, 36.0], Some("VCC")),
        ([20.0, 63.62], [20.0, 70.0], Some("GND")),
        ([20.0, 70.0], [46.0, 70.0], Some("GND")),
        ([46.0, 70.0], [46.0, 47.62], Some("GND")),
        ([52.7, 36.0], [54.0, 36.0], Some("IO1")),
        ([54.0, 36.0], [54.0, 24.0], Some("IO1")),
        ([54.0, 24.0], [57.3, 24.0], Some("IO1")),
        ([52.7, 44.0], [54.0, 44.0], Some("IO2")),
        ([54.0, 44.0], [54.0, 56.0], Some("IO2")),
        ([54.0, 56.0], [57.3, 56.0], Some("IO2")),
    ];
    for (from, to, net) in wires {
        add_wire(handle, AddWireRequest { sheet: None, net: net.map(str::to_string), from, to })?;
    }
    add_text(handle, AddTextRequest { sheet: None, text: "Demo schematic".into(), x: 8.0, y: 78.0 })?;

    for format in ["svg", "pdf"] {
        let target = out.join(format!("demo-schematic.{format}"));
        let report = export(handle, format, &target)?;
        println!("{format}: {report}");
    }
    Ok(())
}
