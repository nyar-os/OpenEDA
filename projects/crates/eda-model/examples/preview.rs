//! Temporary preview driver: builds a demo schematic and exports SVG and PDF.

use std::path::Path;

use open_eda_model::{
    AddComponentRequest, AddJunctionRequest, AddTextRequest, AddWireRequest, CreateRequest, add_component, add_junction,
    add_text, add_wire, create, export,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args().nth(1).unwrap_or_else(|| "preview".into());
    let out = Path::new(&out);
    std::fs::create_dir_all(out)?;

    let created = create(CreateRequest { title: Some("OpenEDA demo sheet".into()), template: Some("blank".into()) })?;
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
        ([36.0, 34.0], [24.0, 34.0], Some("VCC")),
        ([36.0, 40.0], [24.0, 40.0], Some("GND")),
        ([48.0, 34.0], [70.0, 34.0], Some("IO1")),
        ([48.0, 46.0], [70.0, 46.0], Some("IO2")),
        ([24.0, 34.0], [24.0, 52.0], Some("VCC")),
        ([24.0, 40.0], [24.0, 60.0], Some("GND")),
        ([70.0, 34.0], [78.0, 34.0], Some("IO1")),
        ([78.0, 34.0], [78.0, 62.0], Some("IO1")),
    ];
    for (from, to, net) in wires {
        add_wire(handle, AddWireRequest { sheet: None, net: net.map(str::to_string), from, to })?;
    }
    for (x, y) in [(24.0, 34.0), (24.0, 40.0), (70.0, 34.0)] {
        add_junction(handle, AddJunctionRequest { sheet: None, x, y })?;
    }
    add_text(handle, AddTextRequest { sheet: None, text: "Demo schematic".into(), x: 18.0, y: 76.0 })?;

    for format in ["svg", "pdf"] {
        let target = out.join(format!("demo-schematic.{format}"));
        let report = export(handle, format, &target)?;
        println!("{format}: {report}");
    }
    Ok(())
}
