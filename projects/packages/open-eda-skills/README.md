# @wae/open-eda-skills

Agent skills for OpenEDA. The package ships skill documents only; it is not a
runtime library and contains no schematic, board, routing or fabrication
algorithms.

## Status

`skills/open-eda/SKILL.md` records the routing and evidence rules for create,
edit, review and delivery against the implemented document core. The CLI and SDK
behind it create, edit and inspect a `.oeda` document, undo and redo edits, edit
sheets, and project to a netlist, SVG, BOM or PDF. Board layout, DRC, clearance,
power and KiCad/Gerber exchange are not implemented, and the skill says so
instead of inventing commands.

## Layout

```text
packages/open-eda-skills/
├── package.json
├── README.md
└── skills/
    └── open-eda/
        └── SKILL.md
```

The skill must never be shipped claiming methods, commands or flags that the
installed SDK/CLI does not expose. The capability matrix remains the only support
claim; unsupported checks and exchange targets reject with
`capability/not-implemented`.
