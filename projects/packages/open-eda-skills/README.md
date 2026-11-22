# @wae/open-eda-skills

Agent skills for OpenEDA.

```bash
pnpm add @wae/open-eda-skills
```

The package ships skill documents only. Point an agent at:

```text
node_modules/@wae/open-eda-skills/skills/open-eda/SKILL.md
```

The skill records the `open-eda` command surface, preflight steps and evidence
rules for creating, editing, checking and exporting a `.oeda` schematic. It
never claims a method, command or flag that the installed CLI/SDK does not
expose: the capability matrix printed by `open-eda capabilities` remains the
only support claim, and unsupported work returns `capability/not-implemented`.

## Links

- [Product overview](https://github.com/nyar-os/OpenEDA)
- [CLI reference](https://github.com/nyar-os/OpenEDA/blob/master/documentation/cli.md)
- [Package README](https://github.com/nyar-os/OpenEDA/blob/master/projects/packages/open-eda/README.md)

## License

Mozilla Public License 2.0.
