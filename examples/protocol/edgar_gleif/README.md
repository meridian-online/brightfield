# edgar_gleif Protocol fixture — provenance

`arcform.yaml` and `models/*.sql` are vendored **verbatim** from the public
[open-analytics](https://github.com/meridian-online/open-analytics) repository
(MIT licence, same project family) at `datasets/edgar_gleif/` — the arcform
Protocol that builds the EDGAR ↔ GLEIF crosswalk dataset. Only the manifest and
its SQL models are vendored (no data, no build outputs); nothing has been
modified. The fixture exists because it is the real shape a Protocol DAG render
must handle: a 10-file fan-in, a four-quarter parameterised fetch+extract
family, a long-running operator step, a validation gate, and one terminal
Dataset sink (card 0025).
