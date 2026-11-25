use super::*;
use crate::{Component, Design, DesignRules, Junction, LabelScope, NetLabel, NoConnect, Sheet, Wire};
use std::collections::BTreeMap;

fn sample_design() -> Design {
    Design {
        id: "png-design".to_string(),
        title: None,
        rev: String::new(),
        company: String::new(),
        date: String::new(),
        revision: 1,
        units: "mm".to_string(),
        sheets: vec![Sheet {
            id: "sheet-1".to_string(),
            name: "Root".to_string(),
            components: vec![Component {
                id: "component-1".to_string(),
                reference: "R1".to_string(),
                value: "10k".to_string(),
                footprint: String::new(),
                x: 50.0,
                y: 30.0,
                rotation: 0.0,
                pins: vec!["1".to_string(), "2".to_string()],
                pin_types: BTreeMap::new(),
                properties: BTreeMap::new(),
            }],
            nets: Vec::new(),
            wires: vec![Wire { id: "wire-1".to_string(), net: None, from: [10.0, 10.0], to: [90.0, 10.0] }],
            junctions: vec![Junction { x: 50.0, y: 10.0 }],
            no_connects: vec![NoConnect { x: 20.0, y: 45.0, node: None }],
            texts: Vec::new(),
            labels: vec![NetLabel {
                id: "label-1".to_string(),
                text: "VCC".to_string(),
                x: 30.0,
                y: 20.0,
                scope: LabelScope::Global,
                rotation: 0.0,
                justify: "left".to_string(),
            }],
        }],
        buses: Vec::new(),
        variants: Vec::new(),
        hierarchy: Vec::new(),
        rules: DesignRules::default(),
        next_component: 2,
        next_net: 1,
        next_wire: 2,
    }
}

/// Walks the PNG chunk stream and returns `(type, data)` pairs after IHDR.
fn chunks(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    assert_eq!(&bytes[..8], &[137, 80, 78, 71, 13, 10, 26, 10], "the file starts with the PNG signature");
    let mut out = Vec::new();
    let mut offset = 8;
    while offset < bytes.len() {
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().expect("length")) as usize;
        let kind = String::from_utf8(bytes[offset + 4..offset + 8].to_vec()).expect("chunk type");
        let data = bytes[offset + 8..offset + 8 + length].to_vec();
        let crc = u32::from_be_bytes(bytes[offset + 8 + length..offset + 12 + length].try_into().expect("crc"));
        let mut crc_input = Vec::new();
        crc_input.extend_from_slice(&bytes[offset + 4..offset + 8]);
        crc_input.extend_from_slice(&data);
        assert_eq!(crc, crc32(&crc_input), "chunk `{kind}` carries a valid CRC");
        out.push((kind, data));
        offset += 12 + length;
    }
    out
}

#[test]
fn checksum_helpers_match_known_answers() {
    assert_eq!(crc32(b""), 0);
    assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    assert_eq!(crc32(b"IHDR"), 0xA8A1_AE0A);
    assert_eq!(adler32(b""), 1);
    assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
}

#[test]
fn encoder_writes_a_valid_rgb_png() {
    let rgb = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0];
    let bytes = encode_png(2, 2, &rgb);
    let parsed = chunks(&bytes);
    let kinds: Vec<&str> = parsed.iter().map(|(kind, _)| kind.as_str()).collect();
    assert_eq!(kinds, vec!["IHDR", "IDAT", "IEND"]);
    assert_eq!(parsed[0].1, vec![0, 0, 0, 2, 0, 0, 0, 2, 8, 2, 0, 0, 0], "IHDR declares 2x2 RGB8");
    // Stored deflate blocks are trivially decodable: strip headers and compare scanlines.
    let idat = &parsed[1].1;
    assert_eq!(&idat[..2], &[0x78, 0x01], "zlib header for an uncompressed stream");
    let mut raw = Vec::new();
    let mut offset = 2;
    while idat[offset] & 1 == 0 {
        let length = u16::from_le_bytes(idat[offset + 1..offset + 3].try_into().expect("block length")) as usize;
        raw.extend_from_slice(&idat[offset + 5..offset + 5 + length]);
        offset += 5 + length;
    }
    let length = u16::from_le_bytes(idat[offset + 1..offset + 3].try_into().expect("block length")) as usize;
    raw.extend_from_slice(&idat[offset + 5..offset + 5 + length]);
    assert_eq!(raw, vec![0, 255, 0, 0, 0, 255, 0, 0, 0, 0, 255, 255, 255, 0], "filter byte 0 per scanline");
    assert_eq!(u32::from_be_bytes(idat[idat.len() - 4..].try_into().expect("adler")), adler32(&raw));
}

#[test]
fn raster_is_fixed_size_deterministic_and_paints_the_geometry() {
    let design = sample_design();
    let sheets = design.sheets.iter().collect::<Vec<_>>();
    let bytes = render_png(&sheets);
    let parsed = chunks(&bytes);
    let ihdr = &parsed[0].1;
    assert_eq!(&ihdr[..8], &[0, 0, 4, 0, 0, 0, 3, 0], "IHDR declares 1024x768");
    assert_eq!(&ihdr[8..], &[8, 2, 0, 0, 0], "bit depth 8, truecolour, no interlace");
    assert_eq!(bytes, render_png(&sheets), "the same document rasterizes byte-identically");

    // Reconstruct the pixels from the stored blocks and look for the painted colours.
    let idat = &parsed.iter().find(|(kind, _)| kind == "IDAT").expect("IDAT").1;
    let mut raw = Vec::new();
    let mut offset = 2;
    while idat[offset] & 1 == 0 {
        let length = u16::from_le_bytes(idat[offset + 1..offset + 3].try_into().expect("block length")) as usize;
        raw.extend_from_slice(&idat[offset + 5..offset + 5 + length]);
        offset += 5 + length;
    }
    let length = u16::from_le_bytes(idat[offset + 1..offset + 3].try_into().expect("block length")) as usize;
    raw.extend_from_slice(&idat[offset + 5..offset + 5 + length]);
    let stride = (WIDTH as usize) * 3 + 1;
    assert_eq!(raw.len(), stride * HEIGHT as usize, "one filter byte plus one scanline per row");
    let painted = |colour: Rgb| raw.chunks(stride).any(|row| row[1..].chunks(3).any(|pixel| pixel == &colour[..]));
    assert!(painted(WIRE), "the wire is rasterized");
    assert!(painted(JUNCTION), "the junction is rasterized");
    assert!(painted(NO_CONNECT), "the no-connect mark is rasterized");
    assert!(painted(SYMBOL), "the component body is rasterized");
    assert!(painted(GLOBAL_LABEL), "the global label outline is rasterized");
    assert!(painted(FRAME), "the sheet frame is rasterized");
}

#[test]
fn an_empty_design_still_rasterizes() {
    let mut design = sample_design();
    let sheet = &mut design.sheets[0];
    sheet.components.clear();
    sheet.wires.clear();
    sheet.junctions.clear();
    sheet.no_connects.clear();
    sheet.labels.clear();
    let sheets = design.sheets.iter().collect::<Vec<_>>();
    let bytes = render_png(&sheets);
    assert_eq!(&bytes[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    assert_eq!(chunks(&bytes).len(), 3, "IHDR, IDAT and IEND");
}
