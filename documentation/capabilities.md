# Capabilities

`capabilities()` in `projects/crates/eda-model/src/lib.rs` is the only support
claim in this repository. `open-eda capabilities` prints it verbatim. A method
name, CLI flag or file extension is not evidence that a capability exists.

Unsupported work answers with `capability/not-implemented` (exit code `2`)
instead of degrading silently.

## Capability matrix

| Capability | Status | Notes |
| --- | --- | --- |
| `.oeda` create/open/save/edit | supported | sheets, components, nets, buses, wires, junctions, no-connect flags, text, net labels with scope, title block, undo/redo |
| design compare `diff` | supported | read-only report of added, removed and modified sheets, components, nets, wires, buses and BOM variants |
| pin electrical types | supported | per-pin roles, the `electrical` check and the KiCad library pin types |
| reference annotation `annotate` | supported | renumbers per prefix by sheet and placement order; net nodes follow |
| BOM variants and DNP | supported | named DNP designator lists and value overrides applied to `bom`/`bom_json` |
| net classes | supported | name, width, clearance, via and net/bus assignment lists |
| component custom properties | supported | trimmed unique keys, verbatim values; stored in `.oeda`, the JSON projection and KiCad symbol properties |
| checks `erc`, `electrical`, `nets`, `connectivity`, `footprints`, `power` | supported | `drc` and `clearance` are unsupported |
| design `stats` | supported | deterministic per-sheet and design-wide counts; tagged `open-eda-stats` |
| read-only listings `component list`, `net list`, `bus list`, `net-class list`, `wire list`, `junction list`, `no-connect list`, `label list`, `text list`, `sheet list` | supported | document order with 0-based indices where the model edits by index |
| export `net`, `net_csv`, `net_hier`, `spice`, `spice_subckt`, `svg`, `bom`, `bom_json`, `bom_html`, `pdf`, `report`, `json`, `kicad_sch`, `kicad_sym` | supported | read-only projections; `kicad_pcb` and `gbr` are unsupported |
| import `kicad_sch` | supported | symbols, wires, junctions, no-connect flags, text, labels, hierarchical sheet symbols and ports; other constructs are reported as losses |
| hierarchy editing | supported | add/list/remove parent symbol declarations and their ports; child files are never opened |
| import `kicad_sym` | supported | read-only symbol declarations: unit count and each pin's number, name, role and unit |
| export `kicad_sym` | supported | one library symbol per derived library id, with pins and roles; no symbol body is invented |
| KiCad board, Gerber, drill, STEP | unsupported | declared in the capability matrix only |
| interactive schematic/board renderer | unsupported | the preview is an SVG projection |
| DRC and clearance | unsupported | `capability/not-implemented` |
| board layout, routing, simulation | unsupported | not modelled |

## Design model semantics

### Units and identifiers

The document stores millimetre-scale drawing coordinates as written. Component
nodes are `REF.PIN`. Net names, references and pin names must not break the
`NET` line or `REF.PIN` grammar the writers emit; `erc/reference-illegal`,
`erc/pin-name-illegal` and `erc/net-name-illegal` fail inspection instead of
producing an ambiguous file.

### Labels, buses and hierarchy

- A net label is an annotation with `local`, `global` or `hierarchical` scope.
  Labels never derive, rename or merge a net, and a global or hierarchical label
  does not connect nets across sheets.
- A bus is a design-level declaration: a name, a width and optional aliases. A
  bus `DATA` with width 4 expands into `DATA0`..`DATA3` (name plus 0-based index,
  no separator). A declared net with the bus name repeats its nodes into every
  member; a bus without such a net still lists members without nodes. Aliases
  may not collide with a bus name, a member name or another alias.
- A hierarchical sheet symbol is a parent declaration: name, child file,
  placement, size and ports (`input`, `output`, `bidirectional`, `tri_state`,
  `passive`). The model never opens or rewrites the child file and does not check
  that a port matches a hierarchical label there.

### Checks

`erc` reports `erc/duplicate-pin`, `erc/dangling-net`, `erc/duplicate-net-name`,
`erc/single-node-net`, `erc/unconnected-pin`, `erc/floating-wire` and
`erc/unconnected-hierarchical-port`. A no-connect flag may carry a `ref`/`pin`
node; then the `erc` check and the connectivity report treat that pin as
intentionally open. A flag without a node is a coordinate-only marker and
suppresses nothing.

`electrical` reads typed pins only and reports `erc/input-input-conflict`,
`erc/output-output-conflict` and `erc/power-in-unpowered`.

`power` reports `power/driver-conflict` when more than one power-output pin
drives one net, and `power/output-driver-conflict` when a power output shares a
net with a logic output.

`nets` reports `net-bus-member-collision` and `net-bus-alias-collision` so the
netlist never writes one `NET` line twice.

Inspection accepts per-code severity overrides (`error`, `warning`, `info`,
`off`) and waivers by code, optionally narrowed to one object or sheet. Accepted
findings move from `issues` to `waived` with `waived: true`; only active errors
fail the report.

### Reports

- `stats` is a read-only snapshot of the model arrays: per sheet components,
  nets, wires, junctions, labels, no-connect flags and text annotations;
  design-wide the sheet count, BOM rows at default population and declared bus
  count. Tagged `open-eda-stats`.
- `connectivity` lists every net with node count and `REF.PIN` pins, the pins no
  net reaches, per-sheet wire-group and junction totals, and per-sheet label
  counts split by scope. Tagged `open-eda-connectivity`.
- `diff` compares modelled objects, not bytes: sheets by id, components by
  reference, nets by name inside a sheet, buses and BOM variants by name. It
  records field-level changes and is tagged `open-eda-diff`.

## Exchange formats

### KiCad schematic (`kicad_sch`)

The importer and writer cover symbols, wires, junctions, no-connect flags, text
annotations and net labels with scope. The title-block `title`, `date`, `rev` and
`company` fields round-trip; other title-block fields add a loss entry. Custom
component properties round-trip as hidden KiCad symbol properties.

Losses are explicit. Each occurrence adds an entry to `losses`:

| Code | Meaning |
| --- | --- |
| `kicad/unsupported-element` | a construct outside the subset (buses, arcs, images, unknown roots, unknown pin roles, malformed symbols) |
| `kicad/label-scope` | a global or hierarchical label kept as a sheet-local label |
| `kicad/duplicate-reference` | a symbol repeated a reference and was skipped |
| `kicad/missing-reference` | a symbol had no reference and received a fallback |

A hierarchical sheet symbol keeps its name, child file, placement, size and
ports; the child file is never opened. A no-connect flag keeps its placement;
the optional pin attachment is native-only.

### KiCad symbol library (`kicad_sym`)

`library <file>` reads a library without creating a design and reports every
symbol's unit count and each pin's number, name, electrical role and unit.
Constructs outside the subset add `kicad_sym/unsupported-element` losses. The
`kicad_sym` export writes one symbol per derived library id with pins and roles;
it writes no graphic body, value or footprint.

### Netlist and SPICE

- `net` emits `# OpenEDA netlist v1`, `BUS <name> <width>` lines and
  `NET <name>` / `NODE <ref>.<pin>` lines.
- `net_hier` prefixes nets with a sanitized sheet path (`NET Main/VCC`) and emits
  `SHEET <path>` lines.
- `net_csv` emits `Sheet,Net,Ref,Pin` rows with CSV quoting.
- `spice` emits one element line per component by reference prefix (`R`/`C`/`L`
  two-pin, `D` two-pin with model, `Q`/`J` three-pin, `V`/`I` sources, `U`/`X`
  subcircuits, unknown prefixes as generic `X`). An unconnected pin becomes a
  stable `NC_<ref>_<pin>` node and is reported with `spice/open-pin`.
- `spice_subckt` wraps every sheet in `.SUBCKT <name> <ports>` / `.ENDS <name>`
  with legalized, unique names and the same open-pin nodes.

### Presentation and assembly

- `svg` draws the frame, wires, junctions, text and component boxes with
  reference and value labels.
- `bom` groups by value and footprint and emits
  `Comment,Footprint,Designator,Quantity` with compressed designator ranges
  (`R5-R7` for three or more consecutive numbers). A variant filters DNP
  designators and applies value overrides; every referenced designator must
  already be placed.
- `bom_json` and `bom_html` project the same rows as deterministic JSON and a
  self-contained table.
- `pdf` writes a complete one-page A4 landscape vector document.
- `report` writes deterministic Markdown with a design header, per-sheet object
  counts, per-sheet `Net | Nodes | Pins` tables and inspection findings by
  severity.
- `json` writes a read-only `open-eda-projection` of the design header and every
  sheet's objects.

Every file is written through a temporary file and a rename.

## Unsupported boundaries

- No KiCad board, Gerber, drill or STEP import or export.
- No board model, copper, routing, DRC or clearance analysis.
- No schematic simulation; the SPICE output is a netlist only.
- No bus wires, arcs or child-sheet content in the KiCad subset; they are
  reported as losses.
- No interactive schematic or board rendering; the studio and previews are SVG
  or list projections.
- No automatic footprint choice or validation: the footprint text is stored and
  emitted verbatim.
- BOM variants and net classes filter and describe data; they do not change the
  schematic or any board output.
- `diff` does not merge, apply or repair a difference.
- No fabricated previews, sample boards or placeholder reports.
- A format is never claimed because its extension is known.

## Tests

- `projects/crates/eda-model/tests/unit/lib.rs` — create, edit, inspect, save
  and reopen, writers, statistics, labels, variants, properties and KiCad
  round-trips.
- `projects/crates/eda-model/tests/unit/kicad.rs` — parser and writer subset.
- `projects/crates/eda-model/tests/unit/diff.rs` — structural comparison.
- `projects/crates/eda-model/tests/unit/connectivity.rs` — net and pin analysis.
- `projects/packages/open-eda/tests/smoke.test.ts` — CLI/SDK surface, with
  native-addon-dependent cases skipped when no addon is installed.
