# Vendored FineType transforms and displays

`transforms.json` holds, for each FineType label, the DuckDB type and the SQL `transform` its definition carries (`{col}` stands for the column). The silver cast reads it.

`displays.json` holds, for each FineType label whose cast yields a number or a date, the `display` block its definition carries: a d3-format specifier (`number`) or a d3-time-format specifier (`time`), and the flags a specifier cannot say (`money`, `currency`, `fraction`, `bytes`). The blocks are copied under FineType's key names; `src/display.rs` reads them.

- Source: `labels/definitions_*.yaml` in https://github.com/meridian-online/finetype at the tag recorded in each file's `tag` field, which is the tag in `packaging/finetype-pin.env`. FineType publishes no separate displays file, so both come from the definitions.
- Generated: 2026-10-31.
- Regenerate: `scripts/vendor-finetype-transforms.sh`, which writes both files (needs `gh` and python3 with PyYAML installed system-wide or in the active environment; the script ignores the user site), after a bump of the pin. `scripts/check-finetype-pin.sh` refuses either file when its tag differs from the pin.
- Why not `finetype-core`: it links a tokenizer, an HTTP client and a C crypto build for a table of strings.
