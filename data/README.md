# Bundled connectome data

The simulator ships preprocessed data so a fresh checkout or portable release
runs offline without an account, Python packages, or a first-run download.

| File | Purpose |
| --- | --- |
| brain.bin | CSR brain network: 139,255 neurons, 2,656,027 edges; header declares 33,236,616 synapses |
| points.bin | Neuron positions and cell-group annotations for the optional brain panel |
| labels.json | Class and group names |
| meshes.bin | Optional legacy brain anatomy meshes; not loaded by the lightweight viewer or included in portable archives |

The derived files were retained from the existing local drosophila project;
they were not independently regenerated or biologically revalidated for this
release. SHA256SUMS identifies the exact bundled bytes. Run
`python3 tools/verify_data.py` to check them.

Provenance recorded by that project: FlyWire / FAFB v783, distributed by
FlyWire Codex: https://codex.flywire.ai/ . Source tables:
https://storage.googleapis.com/flywire-data/codex/data/fafb/783/ .
Optional mesh provenance and scientific references are in ../CREDITS.md.

The FlyWire-derived data is distributed under CC BY-NC 4.0:
https://creativecommons.org/licenses/by-nc/4.0/ . Preserve attribution and
noncommercial restrictions. The MIT code license does not relicense this data.

To regenerate the network, obtain `coordinates.csv.gz`, `classification.csv.gz`,
`connections.csv.gz`, and `consolidated_cell_types.csv.gz` from the recorded
v783 source into this directory, then run `cargo run --release -- etl`.
Raw CSVs and OBJ inputs are excluded from Git. Anatomy OBJ files are optional.
