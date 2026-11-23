# CLI reference

`open-eda` is a thin shell over the `OpenEDA` session. It parses options, calls
one session or `Design` method, prints the result and returns an exit code.

- Exit `0` — success.
- Exit `1` — every failure: invalid input, an unbuildable design, an IO failure,
  a CLI argument error or a capability the build does not implement. The stable
  error code printed with the message identifies the class.

The design document is the first positional argument unless a command documents
something else. List options take comma-separated values (`--refs R1,C1`).
Run `open-eda --help` for the installed command tree and
`open-eda capabilities` for the support claim.

The CLI opens one sidecar-history session, so `undo` and `redo` work across
separate invocations. History lives in `<file>.history.json` beside the
document and is never written into the document itself.

Every command also accepts the framework-wide `--json` flag. It writes one
`wae.cli-result/v0` envelope to stdout instead of the human text, and help goes
to stderr.

## Create, inspect and compare

| Command | Description |
|---------|-------------|
| `open-eda create --out <file.oeda> [--title <text>] [--rev <text>] [--company <text>] [--date <text>] [--template <name>] [--overwrite]` | Create a native design document |
| `open-eda open <file>` | Validate and report that the document opens |
| `open-eda design set <file> [--title <text>] [--rev <text>] [--company <text>] [--date <text>]` | Update title-block properties; an empty value clears a field |
| `open-eda inspect <file> [--checks erc,electrical,drivers,bus,nets,connectivity,footprints,power,references,clearance] [--severity <code>=<level>] [--waive <code>[@<object>]]` | Run the supported checks |
| `open-eda list <file>` | List sheets, components, nets, wires and buses |
| `open-eda symbols <file>` | Group placed components by derived library id with pin, footprint and missing-footprint counts |
| `open-eda connectivity <file>` | Print the connectivity report |
| `open-eda trace <file> <from> <to>` | Trace the read-only connection path between two `REF.PIN` pins |
| `open-eda stats <file>` | Print deterministic per-sheet and design-wide counts |
| `open-eda diff <before> <after>` | Compare two documents structurally |
| `open-eda annotate <file>` | Renumber references per leading prefix by sheet, then `y` and `x`; report `count`/`changed`, `changes` and skipped references |
| `open-eda translate <file> --dx <n> --dy <n> [--refs <list>] [--wires <list>]` | Move selected components and wires |
| `open-eda undo <file>` / `open-eda redo <file>` | Walk the sidecar history |
| `open-eda capabilities` | Print the machine-readable capability matrix |
| `open-eda gui` | Open the studio window |

### Drivers and library checks

`inspect --checks drivers` reports `erc/undriven-input` when a net reaches typed
input pins but no typed driver, and `erc/unconnected-power-pin` when a typed
power input reaches no net and no no-connect flag names it.

`library-check` prints `library/*` findings with their symbol, pin and unit.
Exit `0` means no error finding (a clean library or warnings only); exit `1`
means an error finding, a missing argument or an unusable file.

### Pin trace

`trace <file> <from> <to>` takes two `REF.PIN` arguments and prints `connected`,
`via` and the hop list (`pin`, `wire`, `junction`, `label`, `sheet-port`). Hops
name the sheet and the 0-based index or object name. `via` is `direct` for the
same pin, `label` when the path needs a label hop, `wire` when nets, wires and
junctions already connect the pins, and `none` when they do not. A disconnected
pair is a successful query (exit `0`); an unknown reference or pin is exit `1`.
`--json` returns the same report as `open-eda-trace`. The command never writes
the document and never opens a child document.

## Components

| Command | Description |
|---------|-------------|
| `open-eda component list <file> [--sheet <id>]` | List components |
| `open-eda component show <file> <ref>` | Report one component's placement, custom properties, pins and the nets that reach each pin |
| `open-eda component add <file> --ref <ref> [--value <text>] --x <n> --y <n> [--rotation <deg>] [--pins <list>] [--footprint <text>] [--sheet <id>]` | Place a component |
| `open-eda component copy <file> --ref <ref> --as <new-ref> [--dx <n>] [--dy <n>] [--sheet <id>]` | Copy placement and metadata without net nodes |
| `open-eda component move <file> --ref <ref> --x <n> --y <n> [--rotation <deg>] [--sheet <id>]` | Move or rotate a component |
| `open-eda component update <file> <ref> [--value <text>] [--footprint <text>] [--x <n>] [--y <n>] [--rotation <deg>] [--sheet <id>]` | Update only the given fields of a component |
| `open-eda component footprint <file> --ref <ref> --footprint <text> [--sheet <id>]` | Assign or clear the footprint |
| `open-eda component pin-type <file> <ref> <pin> <role>` | Type a pin role; add `--clear` to remove it |
| `open-eda component pin add <file> --ref <ref> --pin <pin> [--sheet <id>]` | Add a pin to a placed component |
| `open-eda component pin remove <file> --ref <ref> --pin <pin> [--sheet <id>]` | Remove an unreferenced pin |
| `open-eda component pin rename <file> --ref <ref> --from <pin> --to <pin> [--sheet <id>]` | Rename a pin; net and no-connect nodes follow |
| `open-eda component pin swap <file> --ref <ref> --pin-a <pin> --pin-b <pin> [--sheet <id>]` | Swap the net connections of two pins; each pin takes the other pin's connection |
| `open-eda component property-set <file> --ref <ref> --key <key> --value <text> [--sheet <id>]` | Set a custom property |
| `open-eda component property-list <file> --ref <ref> [--sheet <id>]` | List custom properties |
| `open-eda component property-remove <file> --ref <ref> --key <key> [--sheet <id>]` | Remove a custom property |
| `open-eda component delete <file> --ref <ref> [--sheet <id>]` | Delete a component |

Pin roles: `input`, `output`, `bidirectional`, `tri_state`, `passive`,
`power_in`, `power_out`, `open_collector`, `open_emitter`, `unspecified`.

## Nets, buses and net classes

| Command | Description |
|---------|-------------|
| `open-eda net connect <file> --net <name> --nodes <REF.PIN,...> [--sheet <id>]` | Connect pins to a net |
| `open-eda net disconnect <file> --net <name> --nodes <REF.PIN,...> [--sheet <id>]` | Disconnect pins |
| `open-eda net derive <file>` | Derive nets from connectivity |
| `open-eda net rename <file> --id <net> --name <name> [--sheet <id>]` | Rename a declared net across every sheet; declarations keep their nodes and wires, labels, net-class and bus alias assignments follow; an unknown net is `net/not-found`, a name another net declares is refused, and an already-matching name reports `changed: false` without a new revision |
| `open-eda net merge <file> --target <net> --source <net> [--sheet <id>]` | Merge two nets |
| `open-eda net show <file> <net> [--sheet <id>]` | Report one net's nodes, wires, labels and matching bus and net class |
| `open-eda net list <file> [--sheet <id>]` | List nets and nodes |
| `open-eda net delete <file> --net <name> [--sheet <id>]` | Delete a net |
| `open-eda bus add <file> --name <name> --width <n> [--aliases <list>]` | Declare a bus |
| `open-eda bus update <file> --id <name> [--name <new>] [--width <n>] [--aliases <list>]` | Rename, resize or replace the aliases of a bus; `--aliases ""` clears aliases and an unchanged request reports `changed: false` |
| `open-eda bus list <file>` | List buses, widths and aliases |
| `open-eda bus remove <file> --id <name>` | Remove a bus |
| `open-eda net-class add <file> --name <name> --width <n> [--clearance <n>] [--via <n>] [--nets <list>] [--buses <list>]` | Declare a net class; an existing name is replaced as a whole |
| `open-eda net-class list <file>` | List net classes |
| `open-eda net-class remove <file> --id <name>` | Remove a net class |
| `open-eda reference add <file> --class <class> --prefix <prefix>` | Declare a component class and its reference prefix |
| `open-eda reference assign <file> --ref <ref> --class <class> [--sheet <id>]` | Assign a component to a reference class |
| `open-eda reference list <file>` | List reference classes and assignments |

## Wires, junctions, text and labels

| Command | Description |
|---------|-------------|
| `open-eda wire add <file> --net <name> --from <x,y> --to <x,y> [--sheet <id>]` | Add a wire segment |
| `open-eda wire update <file> --id <wire> --from <x,y> --to <x,y> [--sheet <id>]` | Move a wire endpoint |
| `open-eda wire split <file> --id <wire> --x <n> --y <n> [--sheet <id>]` | Split a wire at a point on the segment |
| `open-eda wire merge <file> --sheet <id> --a <index> --b <index>` | Merge two touching collinear wires of the same net |
| `open-eda wire list <file> [--sheet <id>]` | List wires in document order |
| `open-eda wire delete <file> --id <wire> [--sheet <id>]` | Delete a wire |
| `open-eda junction add <file> --x <n> --y <n> [--sheet <id>]` | Add a junction |
| `open-eda junction list <file> [--sheet <id>]` | List junctions with 0-based indices |
| `open-eda junction move <file> --index <n> --x <n> --y <n> [--sheet <id>]` | Move a junction |
| `open-eda junction delete <file> --index <n> [--sheet <id>]` | Delete a junction |
| `open-eda no-connect add <file> --x <n> --y <n> [--ref <ref> --pin <pin>] [--sheet <id>]` | Add a no-connect flag; with a node it marks the pin deliberately open |
| `open-eda no-connect list <file> [--sheet <id>]` | List no-connect flags |
| `open-eda no-connect delete <file> --index <n> [--sheet <id>]` | Delete a no-connect flag |
| `open-eda text add <file> --text <text> --x <n> --y <n> [--sheet <id>]` | Add a text annotation |
| `open-eda text list <file> [--sheet <id>]` | List text annotations |
| `open-eda text update <file> --index <n> [--text <text>] [--x <n>] [--y <n>] [--sheet <id>]` | Update a text annotation |
| `open-eda text move <file> --index <n> --x <n> --y <n> [--sheet <id>]` | Move a text annotation |
| `open-eda text delete <file> --index <n> [--sheet <id>]` | Delete a text annotation |
| `open-eda label add <file> --text <text> --x <n> --y <n> [--scope local\|global\|hierarchical] [--rotation <deg>] [--justify left\|right\|center] [--sheet <id>]` | Add a net label |
| `open-eda label list <file> [--sheet <id>]` | List net labels |
| `open-eda label update <file> --id <label> [--text <text>] [--x <n>] [--y <n>] [--scope <scope>] [--rotation <deg>] [--justify <justify>] [--sheet <id>]` | Update a net label |
| `open-eda label delete <file> --id <label> [--sheet <id>]` | Delete a net label |

## Sheets and hierarchy

| Command | Description |
|---------|-------------|
| `open-eda sheet add <file> --name <name>` | Add a sheet |
| `open-eda sheet list <file>` | List sheets |
| `open-eda sheet rename <file> --id <sheet> --name <name>` | Rename a sheet |
| `open-eda sheet delete <file> --id <sheet>` | Delete a sheet |
| `open-eda hierarchy add <file> --name <name> --file <child> --x <n> --y <n> [--width <n>] [--height <n>]` | Add a hierarchical sheet symbol declaration |
| `open-eda hierarchy list <file>` | List sheet symbols and ports |
| `open-eda hierarchy check <file>` | Check child `.oeda` documents without changing them |
| `open-eda hierarchy port-add <file> --id <sheet> --name <port> --direction <dir> --x <n> --y <n>` | Add a port declaration |
| `open-eda hierarchy port-remove <file> --id <sheet> --name <port>` | Remove a port declaration |
| `open-eda hierarchy remove <file> --id <sheet>` | Remove a sheet symbol |

Port directions: `input`, `output`, `bidirectional`, `tri_state`, `passive`.

## Variants

| Command | Description |
|---------|-------------|
| `open-eda variant add <file> --name <name> [--dnp <list>] [--values <REF=value,...>]` | Declare a BOM variant |
| `open-eda variant list <file>` | List variants |
| `open-eda variant remove <file> --name <name>` | Remove a variant |

## Import, library and export

| Command | Description |
|---------|-------------|
| `open-eda import <file.kicad_sch> --out <file.oeda> [--overwrite]` | Import the supported KiCad schematic subset |
| `open-eda library <file.kicad_sym> [--pins]` | Report a KiCad symbol library without creating a design; `--pins` prints each pin's unit, number, name and role |
| `open-eda library-check <file.kicad_sym>` | Validate a KiCad symbol library; exit `1` when a finding is an error |
| `open-eda export <file> --format <format> --out <target> [--variant <name>]` | Write a read-only projection |
| `open-eda netlist <file> --out <file.net> [--overwrite]` | Write the flat netlist |

Export formats: `net`, `net_csv`, `net_hier`, `net_class`, `spice`,
`spice_subckt`, `svg`, `bom`, `bom_json`, `bom_html`, `pdf`, `report`, `json`,
`kicad_sch`, `kicad_sym`. `net_class` groups pin rows by declared net class and
adds the pin electrical role. A variant is accepted by `bom`, `bom_json` and
`bom_html` only; other targets reject it instead of ignoring it.

## Worked example

```bash
open-eda create --out blinker.oeda --title "Blinker" --rev A --company Acme
open-eda component add blinker.oeda --ref R1 --value 10k --x 40 --y 30 --pins 1,2
open-eda component add blinker.oeda --ref C1 --value 100n --x 70 --y 30 --pins 1,2
open-eda net connect blinker.oeda --net VCC --nodes R1.1,C1.1
open-eda wire add blinker.oeda --net VCC --from 40,30 --to 70,30
open-eda junction add blinker.oeda --x 55 --y 30
open-eda label add blinker.oeda --text VCC --x 40 --y 30 --scope global
open-eda variant add blinker.oeda --name lite --dnp C1 --values R1=22k
open-eda inspect blinker.oeda --checks erc,electrical,nets,connectivity
open-eda stats blinker.oeda
open-eda export blinker.oeda --format net --out blinker.net
open-eda export blinker.oeda --format bom --variant lite --out blinker-lite.csv
open-eda export blinker.oeda --format svg --out blinker.svg
open-eda export blinker.oeda --format kicad_sch --out blinker.kicad_sch
```
