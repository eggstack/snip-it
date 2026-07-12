# Test Fixtures

TOML snippet library fixtures for the pet-compatibility test suite.

| File | Purpose |
|------|---------|
| `canonical_pet.toml` | Canonical pet format: `[[snippets]]` with Description, Command, Output, Tag. No snip-it-only fields. |
| `snip_it_native.toml` | Native snip-it format: lowercase `[[snippets]]` with all fields including id, timestamps, device_id, deleted. |
| `legacy_uppercase.toml` | Legacy snp format: `[[Snippets]]` with capitalized field names (Description, Command, Output, Tag). |
| `variable_commands.toml` | Snippets exercising variable syntax: `<name>`, `<name=default>`, escaped `\<\>`, nested brackets, spaces in defaults. |
| `edge_cases.toml` | Edge cases: empty description, long command (500+ chars), backslashes, single tag, empty tags array. |
| `empty_library.toml` | Empty but valid TOML (comment only, no snippets). |
| `mixed_field_aliases.toml` | Mixed alias conventions: `name`/`cmd`/`Tags` alongside canonical field names. |
