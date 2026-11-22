# Capabilities

`capabilities()` in `projects/crates/eda-model/src/lib.rs` is the only support
claim in this repository. `open-eda capabilities` prints it verbatim. A method
name, CLI flag or file extension is not evidence that a capability exists.

Unsupported work answers with `capability/not-implemented` (exit code `2`)
instead of degrading silently.

Read-only queries are reported next to `checks` but are not rule checks:
`queries.trace` is `supported` and `inspect --checks` never accepts `trace`.
A check reports findings on one revision; a query returns one deterministic
answer about the document, such as a pin-to-pin path.

## Capability matrix

| Capability | Status | Notes |
| --- | --- | --- |
| `.oeda` create/open/save/edit | supported | sheets, components, nets, buses, wires, junctions, no-connect flags, text, net labels with scope, title block, undo/redo |
| design compare `diff` | supported | read-only report of added, removed and modified sheets, components, nets, wires, buses and BOM variants |
| pin electrical types | supported | per-pin roles, the `electrical` check and the KiCad library pin types |
| reference annotation `annotate` | supported | renumbers per leading-letter prefix by sheet, then `y` and `x`; a reference without a prefix is skipped and reported under `skipped`; net nodes, no-connect attachments and BOM variants follow; one revision, none when unchanged |
| BOM variants and DNP | supported | named DNP designator lists and value overrides applied to `bom`, `bom_json` and `bom_html` |
| net classes | supported | name, width, clearance, via and net/bus assignment lists |
| bus update `bus update` | supported | partial edit of one declared bus by id, name or alias: rename, width change and whole alias-list replacement (an empty list clears aliases); a rename never rewrites nets, net nodes or net-class assignments, and a width change only changes derived member names; a request that leaves the declaration unchanged reports `changed: false` and adds no revision |
| component custom properties | supported | trimmed unique keys, verbatim values; stored in `.oeda`, the JSON projection and KiCad symbol properties |
| checks `erc`, `electrical`, `bus`, `nets`, `connectivity`, `footprints`, `power`, `references`, `clearance` | supported | `drc` is unsupported; `bus` validates design-level bus declarations; `clearance` compares schematic component bodies; `erc` reports one `erc/duplicate-reference` per reference shared by more than one component; `hierarchy` is a separate read-only entry, not an `inspect --checks` value |
| design `stats` | supported | deterministic per-sheet and design-wide counts; tagged `open-eda-stats` |
| pin trace `trace` | supported | read-only shortest hop path between two `REF.PIN` pins; tagged `open-eda-trace`; reported as `queries.trace`, not as an `inspect --checks` check |
| component detail `component show` | supported | read-only detail of one component by reference: placement, value, footprint, custom properties and per-pin nets; tagged `open-eda-component`; pins keep declaration order and net names keep document order |
| net detail `net show` | supported | read-only detail of one declared net by name across every sheet, or one with `--sheet`: per-sheet net id and nodes, wires and labels that reference the name, aggregated node count, and the matching declared bus and net class; tagged `open-eda-net`; an unknown name is `net/not-found` |
| component update `component update` | supported | partial edit of one placed component by reference: only the fields the request names change; one revision per call; `capabilities()` reports `edits.components` |
| read-only listings `component list`, `net list`, `bus list`, `net-class list`, `wire list`, `label list`, `text list`, `sheet list` | supported | document order with 0-based indices where the model edits by index |
| junction and no-connect editing | supported | add a junction by coordinate and remove it by 0-based index; add a no-connect flag with or without a `REF.PIN` node and remove it by index; a duplicate junction coordinate is refused (`junction/duplicate`) and so is a second flag on one pin (`no-connect/duplicate`); `capabilities()` reports `edits.junctions` and `edits.noConnects` |
| component pin editing | supported | add, remove and rename pins after placement; an empty or `REF.PIN`-breaking name is `session/argument-invalid`, a repeated name `pin/duplicate`, a missing pin `pin/not-found` and a removal that a net node or no-connect flag still uses `pin/in-use`; a rename follows every net node, no-connect node and electrical role; `capabilities()` reports `edits.pins` |
| export `net`, `net_csv`, `net_hier`, `spice`, `spice_subckt`, `svg`, `bom`, `bom_json`, `bom_html`, `pdf`, `report`, `json`, `kicad_sch`, `kicad_sym` | supported | read-only projections; `kicad_pcb` and `gbr` are unsupported |
| import `kicad_sch` | supported | symbols, wires, junctions, no-connect flags, text, labels, hierarchical sheet symbols and ports; other constructs are reported as losses |
| hierarchy editing | supported | add/list/remove parent symbol declarations and their ports; editing never reads the child files |
| hierarchy check `hierarchy check` | supported | read-only depth-first walk of `.oeda` children; reports extension, missing file, parse, cycle, depth and port/label mismatches as `hierarchy/*` issues |
| import `kicad_sym` | supported | read-only symbol declarations: unit count and each pin's number, name, role and unit |
| export `kicad_sym` | supported | one library symbol per derived library id, with pins and roles; no symbol body is invented |
| KiCad board, Gerber, drill, STEP | unsupported | declared in the capability matrix only |
| interactive schematic/board renderer | unsupported | the preview is an SVG/PDF projection that draws component bodies, pin stubs on a 2.54 pitch, wires, junctions, labels and text |
| DRC | unsupported | `capability/not-implemented` |
| board layout, routing, simulation | unsupported | not modelled |

## Design model semantics

### Units and identifiers

The document stores millimetre-scale drawing coordinates as written. Component
nodes are `REF.PIN`. Net names, references and pin names must not break the
`NET` line or `REF.PIN` grammar the writers emit; `erc/reference-illegal`,
`erc/pin-name-illegal` and `erc/net-name-illegal` fail inspection instead of
producing an ambiguous file.

### Reference annotation

`annotate` (`design.annotate()` in the SDK, `open-eda annotate <file>`) is the
one editing call that rewrites reference designators. Components are grouped by
the ASCII letters at the start of their reference (`R`, `C`, `U`, `TP`); each
group is walked sheet by sheet, then top to bottom (`y`) and left to right
(`x`), and numbered from 1. The new reference is the group prefix plus that
number, so `R?`/`R3`/`R7` become `R1`/`R2`/`R3` while `C` and `U` count
independently. Characters after the prefix are replaced, so `TP_A` becomes
`TP1`.

Net nodes, no-connect pin attachments and BOM variant `dnp`/`values` entries
follow their component. Component-class assignments are keyed by the stable
component id and therefore stay attached. A reference that does not start with a
letter is skipped, left untouched and listed under `skipped` with its sheet,
component id and current reference; that is not an error. Duplicate references
are refused when they are created (`object/duplicate`) and an illegal reference
is reported by the `references` check (`erc/reference-illegal`), so annotation
never has to invent a prefix for one.

The report is `{count, changed, annotated, changes, skipped}`: `changes` lists
every `from`/`to` pair in deterministic order, and `count`, `changed` and
`annotated` all hold the number of designators that moved. An already annotated
design reports zero, adds no revision and records no history step. A call that
does move designators adds exactly one revision and one history entry.

### Component updates

`component update` (`design.updateComponent()` in the SDK,
`open-eda component update <file> <ref>`) edits one placed component in place.
A request carries only the fields it changes — `value`, `footprint`, `x`, `y`,
`rotation` — and needs at least one of them; every field it does not name keeps
its current value, and pins, pin roles and custom properties are never
rewritten. The footprint is trimmed and an empty footprint clears the
assignment, while the value is stored verbatim. Coordinates and rotation must
be finite numbers (`session/argument-invalid` otherwise), and rotation is
stored in degrees exactly as passed without wrapping, so `450` stays `450`. A
reference no sheet places fails with `object/not-found`. One successful call
adds exactly one revision and one history entry; a request that names no field
is `session/argument-invalid` and leaves the revision unchanged.

### Labels, buses and hierarchy

- A net label is an annotation with `local`, `global` or `hierarchical` scope.
  Labels never derive, rename or merge a net, and a global or hierarchical label
  does not connect nets across sheets. The read-only pin trace honours the
  declared scope while walking a path (a global label pair bridges sheets, a
  local pair does not); that walk never creates, renames or merges a net.
- A bus is a design-level declaration: a name, a width and optional aliases. A
  bus `DATA` with width 4 expands into `DATA0`..`DATA3` (name plus 0-based index,
  no separator). A declared net with the bus name repeats its nodes into every
  member; a bus without such a net still lists members without nodes. Aliases
  may not collide with a bus name, a member name or another alias. `bus update`
  changes the declaration only: a rename leaves nets, net nodes and net-class
  assignments as they are, and a width change only changes the derived member
  names.
- A hierarchical sheet symbol is a parent declaration: name, child file,
  placement, size and ports (`input`, `output`, `bidirectional`, `tri_state`,
  `passive`). Editing never reads or rewrites the child file.
- `hierarchy check` (`checkHierarchy()` in the SDK, `capabilities().checks.hierarchy`)
  walks the declarations depth-first in document order and opens only children
  whose extension is `.oeda`; any other extension is
  `hierarchy/child-format-unsupported` (warning) and is not opened. A resolved
  child that does not exist is `hierarchy/file-missing` (error), and one that
  cannot be read, parsed or validated is `hierarchy/child-invalid` (error).
  A normalised path already in the current chain is `hierarchy/cycle` (error)
  and stops that branch; more than 32 child levels is
  `hierarchy/depth-exceeded` (error) and also stops that branch.
- The ports of each symbol are compared with the hierarchical-scope labels of
  the child's first sheet: a duplicated port name is
  `hierarchy/duplicate-port` (error), a port without a matching label is
  `hierarchy/port-without-label` (error), and a hierarchical label without a
  matching port is `hierarchy/label-without-port` (warning). Each issue carries
  the child path in `sheet`.
- The report is `{format: "open-eda-hierarchy", version: 1, sheets, issues,
  passed}`; `sheets` counts child documents that opened successfully and
  excludes the root, and `passed` is false only when an issue is an error. The
  check never writes a document. A design created in memory and not yet saved
  has no document path, so the check fails with
  `hierarchy/document-path-missing`.

### Checks

`erc` reports `erc/duplicate-pin`, `erc/duplicate-reference`,
`erc/dangling-net`, `erc/duplicate-net-name`, `erc/single-node-net`,
`erc/unconnected-pin`, `erc/floating-wire` and
`erc/unconnected-hierarchical-port`. A reference is design-wide, so components
sharing one reference are grouped across every sheet in document order and a
group with more than one member reports one `erc/duplicate-reference` error at
its first component, listing every component id in that order. The comparison
is exact and case-sensitive, like the other reference lookups. An empty
reference is `component-empty-reference` and is never counted as a shared
designator. A no-connect flag may carry a `ref`/`pin`
node; then the `erc` check and the connectivity report treat that pin as
intentionally open. A flag without a node is a coordinate-only marker and
suppresses nothing. Removing the pin flag restores the finding, so the flag is
the only state that suppresses it.

`electrical` reads typed pins only and reports `erc/input-input-conflict`,
`erc/output-output-conflict` and `erc/power-in-unpowered`.

`power` reports `power/driver-conflict` when more than one power-output pin
drives one net, and `power/output-driver-conflict` when a power output shares a
net with a logic output.

`nets` reports `net-bus-member-collision` and `net-bus-alias-collision` so the
netlist never writes one `NET` line twice.

`bus` validates the design-level bus declarations. A bus name declared twice is
`bus/duplicate-name` (error) on the later declaration, and a bus with width zero
is `bus/empty` (warning) because it expands to no member. A bus whose members
are neither provided by a declared net named the bus or one of its aliases (the
expansion rule) nor by a declared net with the member name is
`bus/member-missing` (error), one finding per bus listing every missing member.
A net claimed by more than one bus — for example `DATA0` as member 0 of bus
`DATA` and as the expansion net of bus `DATA0` — is `bus/shared-net` (error) on
that net. Bus findings are design-level, so `sheetId` is empty and `objectId` is
the bus id; `bus/shared-net` instead carries the net's sheet and id. The check is
read-only and deterministic for one document state.

`clearance` treats each component body as the axis-aligned rectangle the SVG
preview draws: centred on the placement, with `COMPONENT_HALF_WIDTH` and
`COMPONENT_HALF_HEIGHT` as the half extents. Rotation is ignored, so a rotated
component is still compared as an axis-aligned box. Two bodies that only share
an edge are not an overlap. One `clearance/component-overlap` finding (warning,
non-blocking) is reported per overlapping pair of one sheet, in reference order,
and names both references; components on different sheets are never compared.

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
- `trace` (`tracePins()` in the SDK) is the point-to-point query, not a check.
  It takes two `REF.PIN` strings and returns
  `{format: "open-eda-trace", version: 1, from, to, connected, via, hops,
  sheets}`. Hops are `pin`, `wire`, `junction`, `label` or `sheet-port` nodes
  with the sheet name, the 0-based index or object name, and a readable
  `detail`. The report follows the shortest hop count, with ties broken by
  `(sheet, kind, index or name)` order, so one document state always produces
  the same path. A pin traced to itself returns `via: "direct"` with no hops.
  Two pins nothing reaches return `connected: false` and `via: "none"`; that is
  not an error. `via` is `"label"` only when every path needs a label hop, and
  `"wire"` when net membership, wires and junctions already connect the pins.
  Unknown references and pins fail with `trace/component-missing` and
  `trace/pin-missing`. The trace never writes a document and never opens a
  child document.
- `component show` (`componentReport()` in the SDK) returns one component by
  reference: `{format: "open-eda-component", version: 1, sheet: {id, name},
  component: {id, reference, value, footprint, x, y, rotation, properties},
  pins: [{name, nets, noConnect}]}`. Properties are sorted by key; pins keep the
  component's declaration order and each pin's `nets` keeps document order
  without duplicates. A reference no sheet places fails with
  `object/not-found`. The query never writes a document and adds no revision.
- The trace joins objects with the same rules the connectivity check reads: a
  pin joins the wires of its declared net, wires join at coincident endpoints
  or a T-junction, a junction joins every wire it lies on, and a label joins
  the wire it sits on. Labels join by equal text only where their scope allows:
  any pair on one sheet, and `global` labels across the design. A
  hierarchical-scope label also joins a same-named sheet port, because
  hierarchy declarations are document-level; the port lets two hierarchical
  labels of one name meet without opening a child document.
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
ports; import does not read the child file, and only `hierarchy check` opens it.
A no-connect flag keeps its placement; the optional pin attachment is
native-only.

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
- No board model, copper, routing or DRC analysis; the `clearance` check compares
  schematic component bodies, not copper or board geometry.
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
  and reopen, writers, statistics, annotation, labels, variants, properties,
  partial component updates, determinism and KiCad round-trips.
- `projects/crates/eda-model/tests/unit/kicad.rs` — parser and writer subset.
- `projects/crates/eda-model/tests/unit/diff.rs` — structural comparison.
- `projects/crates/eda-model/tests/unit/connectivity.rs` — net and pin analysis.
- `projects/crates/eda-model/tests/unit/bus.rs` — duplicate bus names, missing
  members, a net shared by two buses, zero-width buses, a valid bus and `bus
  update` rename, width, alias replacement and validation.
- `projects/crates/eda-model/tests/unit/clearance.rs` — component-body overlap
  pairs, edge contact, rotation, sheets, severity and waivers.
- `projects/crates/eda-model/tests/unit/pins.rs` — pin add, remove and rename
  validation, reference-safe removal, net/no-connect rewrites, history and
  save/reopen persistence.
- `projects/crates/eda-model/tests/unit/trace.rs` — pin-to-pin trace paths,
  label scope, determinism and read-only behavior.
- `projects/packages/open-eda/tests/smoke.test.ts` — CLI/SDK surface, with
  native-addon-dependent cases skipped when no addon is installed.
