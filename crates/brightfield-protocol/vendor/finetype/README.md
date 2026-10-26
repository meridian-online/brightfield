# Vendored FineType transforms

`transforms.json` holds, for each FineType label, the DuckDB type and the SQL `transform` its definition carries (`{col}` stands for the column). The silver cast reads it.

- Source: `labels/definitions_*.yaml` in https://github.com/meridian-online/finetype at the tag recorded in the file's `tag` field, which is the tag in `packaging/finetype-pin.env`.
- Generated: 2026-10-26.
- Regenerate: `scripts/vendor-finetype-transforms.sh` (needs `gh` and python3 with PyYAML), after a bump of the pin. `scripts/check-finetype-pin.sh` refuses a file whose tag differs from the pin.
- Why not `finetype-core`: it links a tokenizer, an HTTP client and a C crypto build for a table of strings.
