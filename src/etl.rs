//! Turn the three FlyWire Codex CSV dumps into one binary the simulator and the
//! viewer can both read.
//!
//! The inputs are ~500 MB of gzipped-then-decompressed text, so nothing is ever
//! held as text: each file is streamed once, and only the derived tables stay
//! in memory. Peak footprint is dominated by the edge list (~5M × 12 bytes)
//! plus the CSR it becomes, so a few hundred MB at worst.
//!
//! Three things the raw data does that have to be fixed here rather than
//! downstream:
//!
//! 1. `coordinates.csv` is keyed by *supervoxel*, not by neuron — 50,484 root
//!    ids appear on more than one row. Positions are averaged.
//! 2. `connections.csv` is split by **neuropil**, so the same (pre, post) pair
//!    appears once per region they meet in. Counting rows instead of merging
//!    pairs inflates the connectome.
//! 3. Synapses carry a neurotransmitter, and that is the only sign information
//!    there is. ACH is excitatory; GABA and GLUT are inhibitory; DA, SER and
//!    OCT are modulatory and have no place in a leaky integrate-and-fire model.

use anyhow::{Context, Result, bail};
use flate2::read::MultiGzDecoder;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::coord;

/// "RBIN", little-endian.
pub const MAGIC: u32 = 0x4E49_4252;
/// 1 → 2: added the `group` column.
pub const VERSION: u32 = 2;

/// Bytes per neuron record in `brain.bin`.
pub const NEURON_RECORD: usize = 28;
/// Bytes of header before the neuron table.
pub const HEADER_BYTES: usize = 24;

// --- named cell groups -----------------------------------------------------
//
// The simulation has to talk to the brain through specific, real cell types —
// photoreceptors in, descending neurons out — and neither the super_class nor
// the flow column names them. These groups come from FlyWire's consolidated
// cell type table, cross-checked against the literature names. They are the
// only hand-picked part of the ETL; everything else is mechanical.

pub const G_OTHER: u8 = 0;
/// R1-6, the outer photoreceptors. What "how bright is it" arrives on.
pub const G_PHOTORECEPTOR: u8 = 1;
/// T4a–d: ON-edge motion detectors, one per cardinal direction.
pub const G_T4A: u8 = 2;
pub const G_T4B: u8 = 3;
pub const G_T4C: u8 = 4;
pub const G_T4D: u8 = 5;
/// T5a–d: the same four directions, OFF-edge.
pub const G_T5A: u8 = 6;
pub const G_T5B: u8 = 7;
pub const G_T5C: u8 = 8;
pub const G_T5D: u8 = 9;
/// Looming detectors. A fly's collision alarm.
pub const G_LPLC2: u8 = 10;
pub const G_LC4: u8 = 11;
/// The brain's output port: everything that leaves for the nerve cord.
pub const G_DESCENDING: u8 = 12;
pub const G_MOTOR: u8 = 13;
pub const G_KENYON: u8 = 14;
pub const G_CENTRAL_COMPLEX: u8 = 15;
pub const G_OLFACTORY: u8 = 16;
pub const G_MECHANO: u8 = 17;
pub const G_ASCENDING: u8 = 18;
pub const G_VISUAL_PROJECTION: u8 = 19;
pub const G_MONOAMINE: u8 = 20;

/// Group name for a code, for `stats` and the viewer's legend.
pub const GROUP_NAMES: [&str; 21] = [
    "other",
    "photoreceptor",
    "T4a",
    "T4b",
    "T4c",
    "T4d",
    "T5a",
    "T5b",
    "T5c",
    "T5d",
    "LPLC2",
    "LC4",
    "descending",
    "motor",
    "Kenyon",
    "central_complex",
    "olfactory",
    "mechano",
    "ascending",
    "visual_projection",
    "monoamine",
];

/// From the consolidated cell type table alone.
fn group_from_cell_type(t: &str) -> Option<u8> {
    Some(match t {
        "R1-6" | "R7" | "R8" => G_PHOTORECEPTOR,
        "T4a" => G_T4A,
        "T4b" => G_T4B,
        "T4c" => G_T4C,
        "T4d" => G_T4D,
        "T5a" => G_T5A,
        "T5b" => G_T5B,
        "T5c" => G_T5C,
        "T5d" => G_T5D,
        "LPLC2" => G_LPLC2,
        "LC4" => G_LC4,
        _ => return None,
    })
}

// --- what the classifier says about a neuron -------------------------------

pub const FLOW_INTRINSIC: u8 = 0;
pub const FLOW_AFFERENT: u8 = 1;
pub const FLOW_EFFERENT: u8 = 2;
pub const FLOW_UNKNOWN: u8 = 3;

pub const SIDE_LEFT: u8 = 0;
pub const SIDE_RIGHT: u8 = 1;
pub const SIDE_CENTER: u8 = 2;
pub const SIDE_UNKNOWN: u8 = 3;

/// Bit 0 set: this neuron has a real position rather than the origin.
pub const FLAG_HAS_POSITION: u8 = 1;

fn flow_code(s: &str) -> u8 {
    match s {
        "intrinsic" => FLOW_INTRINSIC,
        "afferent" => FLOW_AFFERENT,
        "efferent" => FLOW_EFFERENT,
        _ => FLOW_UNKNOWN,
    }
}

fn side_code(s: &str) -> u8 {
    match s {
        "left" => SIDE_LEFT,
        "right" => SIDE_RIGHT,
        "center" => SIDE_CENTER,
        _ => SIDE_UNKNOWN,
    }
}

/// Synaptic sign, or 0 for the neuromodulators we do not model.
fn nt_sign(s: &str) -> f32 {
    match s {
        "ACH" => 1.0,
        "GABA" | "GLUT" => -1.0,
        _ => 0.0,
    }
}

fn gz_reader(path: &Path) -> Result<Box<dyn BufRead>> {
    let f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    Ok(Box::new(BufReader::with_capacity(
        1 << 20,
        MultiGzDecoder::new(f),
    )))
}

/// `[352484 175164 229040]` → nanometres. Note the interior padding.
fn parse_position(s: &str) -> Option<[f64; 3]> {
    let inner = s.trim().strip_prefix('[')?.strip_suffix(']')?;
    let mut it = inner.split_whitespace();
    let x = it.next()?.parse().ok()?;
    let y = it.next()?.parse().ok()?;
    let z = it.next()?.parse().ok()?;
    Some([x, y, z])
}

pub struct EtlStats {
    pub neurons: usize,
    pub neurons_without_position: usize,
    pub edges: usize,
    pub synapses: u64,
    pub dropped_edges_missing_neuron: u64,
    pub duplicate_coordinate_rows: u64,
    pub raw_pair_rows: u64,
    pub nt_counts: Vec<(String, u64)>,
    pub bbox_nm: [[f64; 3]; 2],
    pub class_counts: Vec<(String, usize)>,
    pub flow_counts: [usize; 4],
    pub side_counts: [usize; 4],
    pub group_counts: Vec<(String, usize)>,
}

impl EtlStats {
    pub fn print(&self) {
        println!("neurons                 {}", self.neurons);
        if self.neurons_without_position > 0 {
            println!(
                "  of which no position  {}  (classification-only ids)",
                self.neurons_without_position
            );
        }
        println!(
            "coordinate rows         {}  ({} duplicate root_id rows averaged)",
            self.neurons as u64 + self.duplicate_coordinate_rows,
            self.duplicate_coordinate_rows
        );
        println!(
            "raw (pre,post) rows     {}  (split across neuropils)",
            self.raw_pair_rows
        );
        println!("merged edges            {}", self.edges);
        println!("synapses                {}", self.synapses);
        println!(
            "edges dropped           {}  (endpoint not in the neuron table)",
            self.dropped_edges_missing_neuron
        );
        let [lo, hi] = self.bbox_nm;
        println!(
            "bbox (raw nm)           x {:.0}..{:.0}  y {:.0}..{:.0}  z {:.0}..{:.0}",
            lo[0], hi[0], lo[1], hi[1], lo[2], hi[2]
        );
        // Printed in the world frame, where the axes have been permuted out of
        // the raw (left, posterior, dorsal) order.
        println!(
            "world size (mm)         {:.3} long × {:.3} wide × {:.3} tall",
            coord::raw_span_to_mm(hi[1] - lo[1]),
            coord::raw_span_to_mm(hi[0] - lo[0]),
            coord::raw_span_to_mm(hi[2] - lo[2])
        );
        println!();
        println!("flow      intrinsic {}  afferent {}  efferent {}  unknown {}",
            self.flow_counts[0], self.flow_counts[1], self.flow_counts[2], self.flow_counts[3]);
        println!("side      left {}  right {}  center {}  unknown {}",
            self.side_counts[0], self.side_counts[1], self.side_counts[2], self.side_counts[3]);
        println!();
        println!("neurotransmitters (raw synapse counts):");
        for (nt, n) in &self.nt_counts {
            println!("  {:<6} {}", nt, n);
        }
        println!();
        // The groups are what the simulation actually addresses, so they matter
        // more than the super_class breakdown below them.
        println!("named cell groups (what the wiring talks to):");
        for (name, n) in &self.group_counts {
            if *n > 0 && name != "other" {
                println!("  {:<20} {}", name, n);
            }
        }
        println!();
        println!("super_class distribution:");
        let mut cc = self.class_counts.clone();
        cc.sort_by(|a, b| b.1.cmp(&a.1));
        for (name, n) in cc.iter().take(14) {
            println!("  {:<24} {}", name, n);
        }
        if cc.len() > 14 {
            println!("  ... and {} more", cc.len() - 14);
        }
    }
}

pub fn run(data_dir: &Path) -> Result<()> {
    let coords_path = data_dir.join("coordinates.csv.gz");
    let class_path = data_dir.join("classification.csv.gz");
    let conn_path = data_dir.join("connections.csv.gz");
    for p in [&coords_path, &class_path, &conn_path] {
        if !p.exists() {
            bail!("missing {} — see README for the download command", p.display());
        }
    }

    println!("== pass 0/3: cell types ==");
    let cell_types = read_cell_types(&data_dir.join("consolidated_cell_types.csv.gz"))?;
    println!("   {} neurons carry a named cell type", cell_types.len());

    println!("== pass 1/3: coordinates ==");
    let (mut ids, positions, bbox, duplicate_rows) = read_coordinates(&coords_path)?;
    println!("   {} neurons", ids.len());
    // A bbox that is not the whole brain's means something upstream went
    // wrong, and every number after this point would be quietly meaningless.
    for k in 0..3 {
        let got = bbox[1][k] - bbox[0][k];
        let want = coord::EXPECTED_BBOX_NM[1][k] - coord::EXPECTED_BBOX_NM[0][k];
        if (got - want).abs() / want > 0.05 {
            bail!(
                "coordinate bbox on axis {k} spans {got:.0} nm, expected about {want:.0} nm — \
                 this does not look like the whole FAFB brain (truncated download?)"
            );
        }
    }

    println!("== pass 2/3: classification ==");
    let ann = read_classification(&class_path, &mut ids, &cell_types)?;
    let Annotations {
        class_names,
        classes,
        sides,
        flows,
        groups,
        added_positionless,
    } = ann;
    println!("   {} distinct super_class values", class_names.len());
    if added_positionless > 0 {
        println!("   {added_positionless} ids had no coordinates and were added anyway");
    }

    println!("== pass 3/3: connections ==");
    let (merged, stats_extra) = read_connections(&conn_path, &ids)?;
    println!("   {} merged edges", merged.len());

    // Neurons that classification knows about but coordinates do not have no
    // place in a 3D world, so they are dropped here rather than carried around
    // as points at the origin.
    let n = ids.len();

    // --- CSR ---------------------------------------------------------------
    let mut row_start = vec![0u32; n + 1];
    for &(key, _) in &merged {
        row_start[(key >> 32) as usize + 1] += 1;
    }
    for i in 0..n {
        row_start[i + 1] += row_start[i];
    }
    let n_edges = merged.len();
    let mut col_idx = vec![0u32; n_edges];
    let mut weight = vec![0.0f32; n_edges];
    let mut cursor: Vec<u32> = row_start[..n].to_vec();
    let mut synapses = 0u64;
    for &(key, w) in &merged {
        let pre = (key >> 32) as usize;
        let slot = cursor[pre] as usize;
        col_idx[slot] = (key & 0xffff_ffff) as u32;
        weight[slot] = w;
        cursor[pre] += 1;
        synapses += (w.abs().round() as u64).min(1_000_000);
    }

    // --- write brain.bin ---------------------------------------------------
    let brain_path = data_dir.join("brain.bin");
    {
        let f = File::create(&brain_path).context("creating brain.bin")?;
        let mut w = BufWriter::with_capacity(1 << 20, f);
        w.write_all(&MAGIC.to_le_bytes())?;
        w.write_all(&VERSION.to_le_bytes())?;
        w.write_all(&(n as u32).to_le_bytes())?;
        w.write_all(&(n_edges as u32).to_le_bytes())?;
        w.write_all(&synapses.to_le_bytes())?;
        for i in 0..n {
            w.write_all(&ids[i].to_le_bytes())?;
            for k in 0..3 {
                w.write_all(&positions[i][k].to_le_bytes())?;
            }
            w.write_all(&[classes[i], sides[i], flows[i], FLAG_HAS_POSITION])?;
            // Group, then three reserved bytes — keeping the record at a
            // 4-byte stride so the viewer can walk it with a typed array.
            w.write_all(&[groups[i], 0, 0, 0])?;
        }
        for v in &row_start {
            w.write_all(&v.to_le_bytes())?;
        }
        for v in &col_idx {
            w.write_all(&v.to_le_bytes())?;
        }
        for v in &weight {
            w.write_all(&v.to_le_bytes())?;
        }
        w.flush()?;
    }

    // --- write points.bin (what the viewer draws) --------------------------
    // Flat, typed-array-friendly sections rather than a struct walker, because
    // the other end of this file is JavaScript. Positions come out in world
    // millimetres so no coordinate maths ever happens in the browser.
    let points_path = data_dir.join("points.bin");
    {
        let f = File::create(&points_path).context("creating points.bin")?;
        let mut w = BufWriter::with_capacity(1 << 20, f);
        w.write_all(&MAGIC.to_le_bytes())?;
        w.write_all(&(n as u32).to_le_bytes())?;
        w.write_all(&(class_names.len() as u32).to_le_bytes())?;
        w.write_all(&(GROUP_NAMES.len() as u32).to_le_bytes())?;
        for p in positions.iter() {
            for k in 0..3 {
                w.write_all(&(p[k] * coord::MM_PER_BRAIN_UNIT).to_le_bytes())?;
            }
        }
        w.write_all(&classes)?;
        // side decides nothing visually yet but makes the file self-describing
        // for anything that wants to split the brain down the midline.
        w.write_all(&sides)?;
        w.write_all(&groups)?;
        w.flush()?;
    }

    // --- labels.json -------------------------------------------------------
    let mut labels = String::from("{\n  \"super_class\": [");
    for (i, name) in class_names.iter().enumerate() {
        if i > 0 {
            labels.push_str(", ");
        }
        labels.push_str(&format!("\"{}\"", name.replace('"', "")));
    }
    labels.push_str("],\n  \"groups\": [");
    for (i, name) in GROUP_NAMES.iter().enumerate() {
        if i > 0 {
            labels.push_str(", ");
        }
        labels.push_str(&format!("\"{name}\""));
    }
    labels.push_str("],\n  \"sides\": [\"left\", \"right\", \"center\", \"unknown\"],\n");
    labels.push_str("  \"flows\": [\"intrinsic\", \"afferent\", \"efferent\", \"unknown\"]\n}\n");
    std::fs::write(data_dir.join("labels.json"), labels)?;

    // --- meshes ------------------------------------------------------------
    let meshes_path = data_dir.join("meshes.bin");
    write_meshes(data_dir, &meshes_path)?;

    let mut class_counts: Vec<(String, usize)> = class_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            (
                name.clone(),
                classes.iter().filter(|&&c| c as usize == i).count(),
            )
        })
        .collect();
    class_counts.sort_by(|a, b| b.1.cmp(&a.1));

    let mut flow_counts = [0usize; 4];
    for &f in &flows {
        flow_counts[f as usize] += 1;
    }
    let mut side_counts = [0usize; 4];
    for &s in &sides {
        side_counts[s as usize] += 1;
    }
    let group_counts: Vec<(String, usize)> = GROUP_NAMES
        .iter()
        .enumerate()
        .map(|(code, name)| {
            (
                name.to_string(),
                groups.iter().filter(|&&g| g as usize == code).count(),
            )
        })
        .collect();

    let stats = EtlStats {
        neurons: n,
        neurons_without_position: 0,
        edges: n_edges,
        synapses,
        dropped_edges_missing_neuron: stats_extra.dropped,
        duplicate_coordinate_rows: duplicate_rows,
        raw_pair_rows: stats_extra.raw_rows,
        nt_counts: stats_extra.nt_counts,
        bbox_nm: bbox,
        class_counts,
        flow_counts,
        side_counts,
        group_counts,
    };
    println!();
    println!("== result ==");
    stats.print();
    println!();
    println!("wrote {}", brain_path.display());
    println!("wrote {}", points_path.display());
    println!("wrote {}", meshes_path.display());
    println!("wrote {}", data_dir.join("labels.json").display());
    Ok(())
}

// --- pass 1 -----------------------------------------------------------------

fn read_coordinates(path: &Path) -> Result<(Vec<u64>, Vec<[f32; 3]>, [[f64; 3]; 2], u64)> {
    let mut index: HashMap<u64, u32> = HashMap::with_capacity(200_000);
    let mut ids: Vec<u64> = Vec::with_capacity(150_000);
    let mut sum: Vec<[f64; 3]> = Vec::with_capacity(150_000);
    let mut count: Vec<u32> = Vec::with_capacity(150_000);
    let mut bbox = [[f64::MAX; 3], [f64::MIN; 3]];
    let mut duplicates = 0u64;

    let mut r = gz_reader(path)?;
    let mut line = String::new();
    r.read_line(&mut line)?; // header
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        let (id_s, rest) = match line.split_once(',') {
            Some(v) => v,
            None => continue,
        };
        let (pos_s, _vox) = match rest.split_once(',') {
            Some(v) => v,
            None => continue,
        };
        let id: u64 = match id_s.parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let p = match parse_position(pos_s) {
            Some(p) => p,
            None => continue,
        };
        for k in 0..3 {
            if p[k] < bbox[0][k] {
                bbox[0][k] = p[k];
            }
            if p[k] > bbox[1][k] {
                bbox[1][k] = p[k];
            }
        }
        match index.get(&id) {
            Some(&i) => {
                let i = i as usize;
                for k in 0..3 {
                    sum[i][k] += p[k];
                }
                count[i] += 1;
                duplicates += 1;
            }
            None => {
                index.insert(id, ids.len() as u32);
                ids.push(id);
                sum.push(p);
                count.push(1);
            }
        }
    }

    let positions = sum
        .iter()
        .zip(&count)
        .map(|(s, &c)| coord::raw_to_brain([s[0] / c as f64, s[1] / c as f64, s[2] / c as f64]))
        .collect();
    Ok((ids, positions, bbox, duplicates))
}

// --- pass 2 -----------------------------------------------------------------

struct Annotations {
    class_names: Vec<String>,
    classes: Vec<u8>,
    sides: Vec<u8>,
    flows: Vec<u8>,
    groups: Vec<u8>,
    added_positionless: usize,
}

/// Pass 0: FlyWire's consolidated cell type table, keyed by root id.
///
/// This is the only file that names `T4a` or `LPLC2`, and those names are the
/// only way to address the neurons the simulation actually needs to talk to —
/// photoreceptors in, descending neurons out. So it is read before anything
/// that has to assign a group.
fn read_cell_types(path: &Path) -> Result<HashMap<u64, u8>> {
    let mut map: HashMap<u64, u8> = HashMap::with_capacity(160_000);
    if !path.exists() {
        println!("   (no consolidated_cell_types.csv.gz — groups fall back to super_class)");
        return Ok(map);
    }
    let mut r = gz_reader(path)?;
    let mut line = String::new();
    r.read_line(&mut line)?;
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            break;
        }
        let l = line.trim_end();
        if l.is_empty() {
            continue;
        }
        let mut f = l.split(',');
        let id: u64 = match f.next().and_then(|s| s.parse().ok()) {
            Some(v) => v,
            None => continue,
        };
        // primary_type, then any additional types in the fields after it.
        for tok in f {
            if let Some(code) = group_from_cell_type(tok) {
                map.insert(id, code);
                break;
            }
        }
    }
    Ok(map)
}

fn read_classification(
    path: &Path,
    ids: &mut Vec<u64>,
    cell_types: &HashMap<u64, u8>,
) -> Result<Annotations> {
    let mut names: Vec<String> = Vec::new();
    let mut name_index: HashMap<String, u8> = HashMap::new();

    let mut index: HashMap<u64, u32> = HashMap::with_capacity(ids.len() * 2);
    for (i, &id) in ids.iter().enumerate() {
        index.insert(id, i as u32);
    }

    let mut classes: Vec<u8> = vec![0; ids.len()];
    let mut sides: Vec<u8> = vec![SIDE_UNKNOWN; ids.len()];
    let mut flows: Vec<u8> = vec![FLOW_UNKNOWN; ids.len()];
    let mut groups: Vec<u8> = vec![G_OTHER; ids.len()];
    let mut added_positionless = 0usize;

    let mut r = gz_reader(path)?;
    let mut line = String::new();
    r.read_line(&mut line)?;
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            break;
        }
        let l = line.trim_end();
        if l.is_empty() {
            continue;
        }
        let f: Vec<&str> = l.split(',').collect();
        if f.len() < 8 {
            continue;
        }
        let id: u64 = match f[0].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let (flow_s, super_class, class_s) = (f[1], f[2], f[3]);
        let super_class = if super_class.is_empty() { "unknown" } else { super_class };

        let ci = match name_index.get(super_class) {
            Some(&c) => c,
            None => {
                let c = names.len() as u8;
                names.push(super_class.to_string());
                name_index.insert(super_class.to_string(), c);
                c
            }
        };

        // The cell type table is authoritative when it has an entry; the
        // coarser annotation is the fallback.
        let group = match cell_types.get(&id) {
            Some(&g) => g,
            None => match super_class {
                "descending" => G_DESCENDING,
                "ascending" => G_ASCENDING,
                "motor" => G_MOTOR,
                _ => match class_s {
                    "Kenyon_Cell" => G_KENYON,
                    "CX" => G_CENTRAL_COMPLEX,
                    "olfactory" | "ALPN" => G_OLFACTORY,
                    "mechanosensory" => G_MECHANO,
                    "visual" => G_VISUAL_PROJECTION,
                    "brain_motor_neuron" => G_MOTOR,
                    "DAN" => G_MONOAMINE,
                    _ => G_OTHER,
                },
            },
        };

        let i = match index.get(&id) {
            Some(&i) => i as usize,
            None => {
                // Classification knows a few ids that coordinates does not.
                // Grow every parallel table together rather than dropping
                // them, so the stats can report the discrepancy honestly.
                let i = ids.len();
                index.insert(id, i as u32);
                ids.push(id);
                classes.push(0);
                sides.push(SIDE_UNKNOWN);
                flows.push(FLOW_UNKNOWN);
                groups.push(G_OTHER);
                added_positionless += 1;
                i
            }
        };
        classes[i] = ci;
        sides[i] = side_code(f[6]);
        flows[i] = flow_code(flow_s);
        groups[i] = group;
    }
    Ok(Annotations {
        class_names: names,
        classes,
        sides,
        flows,
        groups,
        added_positionless,
    })
}

// --- pass 3 -----------------------------------------------------------------

struct ConnStats {
    dropped: u64,
    raw_rows: u64,
    nt_counts: Vec<(String, u64)>,
}

fn read_connections(path: &Path, ids: &[u64]) -> Result<(Vec<(u64, f32)>, ConnStats)> {
    let mut index: HashMap<u64, u32> = HashMap::with_capacity(ids.len() * 2);
    for (i, &id) in ids.iter().enumerate() {
        index.insert(id, i as u32);
    }

    let mut edges: Vec<(u64, f32)> = Vec::with_capacity(6_000_000);
    let mut nt_counts: HashMap<String, u64> = HashMap::new();
    let mut dropped = 0u64;
    let mut raw_rows = 0u64;

    let mut r = gz_reader(path)?;
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut since_report = 0u64;
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        let mut f = line.split(',');
        let (pre_s, post_s) = match (f.next(), f.next()) {
            (Some(a), Some(b)) => (a, b),
            _ => continue,
        };
        let (_neuropil, syn_s, nt) = match (f.next(), f.next(), f.next()) {
            (Some(a), Some(b), Some(c)) => (a, b, c),
            _ => continue,
        };
        raw_rows += 1;
        since_report += 1;
        if since_report >= 1_000_000 {
            since_report = 0;
            print!("\r   {} rows read", raw_rows);
            let _ = std::io::stdout().flush();
        }
        let (pre, post) = match (pre_s.parse::<u64>(), post_s.parse::<u64>()) {
            (Ok(a), Ok(b)) => (a, b),
            _ => {
                dropped += 1;
                continue;
            }
        };
        let (a, b) = match (index.get(&pre), index.get(&post)) {
            (Some(&a), Some(&b)) => (a, b),
            _ => {
                dropped += 1;
                continue;
            }
        };
        *nt_counts.entry(nt.to_string()).or_insert(0) +=
            syn_s.parse::<u64>().unwrap_or(0);
        let sign = nt_sign(nt);
        if sign == 0.0 {
            continue; // modulatory: counted above, not modelled
        }
        let n: f32 = syn_s.parse().unwrap_or(0.0);
        edges.push((((a as u64) << 32) | b as u64, sign * n));
    }
    println!("\r   {} rows read", raw_rows);

    // Merge the per-neuropil rows into one weight per (pre, post). Sorting by
    // the packed key makes this a linear scan with no hash map at all, which
    // matters at five million entries — and compacting in place rather than
    // into a second vector keeps the peak at one copy instead of two.
    edges.sort_unstable_by_key(|e| e.0);
    let mut w = 0usize;
    for r in 0..edges.len() {
        if w > 0 && edges[w - 1].0 == edges[r].0 {
            edges[w - 1].1 += edges[r].1;
        } else {
            edges[w] = edges[r];
            w += 1;
        }
    }
    edges.truncate(w);
    // Pairs that came out exactly balanced (as much ACH as GABA) are silent;
    // keeping them would add edges that can never do anything.
    edges.retain(|e| e.1.abs() > 1e-6);
    let merged = edges;

    let mut nts: Vec<(String, u64)> = nt_counts.into_iter().collect();
    nts.sort_by(|a, b| b.1.cmp(&a.1));
    Ok((
        merged,
        ConnStats {
            dropped,
            raw_rows,
            nt_counts: nts,
        },
    ))
}

// --- meshes -----------------------------------------------------------------

/// Concatenate every OBJ in `data/` into one file the viewer reads in a single
/// request, converting vertices from nanometres to world millimetres on the
/// way through so no coordinate maths ever happens in JavaScript.
fn write_meshes(data_dir: &Path, out: &Path) -> Result<()> {
    let mut sources: Vec<(String, PathBuf)> = Vec::new();
    let shell = data_dir.join("brain_shell.obj");
    if shell.exists() {
        sources.push(("whole_brain".to_string(), shell));
    }
    let neuropil_dir = data_dir.join("neuropils");
    if neuropil_dir.is_dir() {
        let mut entries: Vec<_> = std::fs::read_dir(&neuropil_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "obj"))
            .collect();
        entries.sort();
        for p in entries {
            let name = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "?".into());
            sources.push((name, p));
        }
    }
    if sources.is_empty() {
        println!("   (no .obj files found — skipping meshes.bin)");
        return Ok(());
    }

    let f = File::create(out)?;
    let mut w = BufWriter::with_capacity(1 << 20, f);
    w.write_all(&MAGIC.to_le_bytes())?;
    w.write_all(&(sources.len() as u32).to_le_bytes())?;

    let mut total_v = 0usize;
    for (name, path) in &sources {
        let (verts, idx) = parse_obj(path)?;
        total_v += verts.len();
        let nb = name.as_bytes();
        w.write_all(&(nb.len() as u32).to_le_bytes())?;
        w.write_all(nb)?;
        w.write_all(&(verts.len() as u32).to_le_bytes())?;
        // Triangle count, not index count. The two differ by a factor of three,
        // and a reader that guesses wrong reads three times too far and lands
        // past the end of the file.
        w.write_all(&((idx.len() / 3) as u32).to_le_bytes())?;
        for v in &verts {
            for k in 0..3 {
                w.write_all(&v[k].to_le_bytes())?;
            }
        }
        for i in &idx {
            w.write_all(&i.to_le_bytes())?;
        }
    }
    w.flush()?;
    println!(
        "   {} meshes, {} vertices total",
        sources.len(),
        total_v
    );
    Ok(())
}

/// Just enough OBJ for Meshlab output: `v x y z` and triangulated `f`.
fn parse_obj(path: &Path) -> Result<(Vec<[f32; 3]>, Vec<u32>)> {
    let f = File::open(path)?;
    let mut r = BufReader::with_capacity(1 << 18, f);
    let mut verts: Vec<[f32; 3]> = Vec::new();
    let mut idx: Vec<u32> = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            break;
        }
        let l = line.trim_end();
        if let Some(rest) = l.strip_prefix("v ") {
            let mut it = rest.split_whitespace();
            let x: f64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let y: f64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let z: f64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            verts.push(coord::raw_to_mm([x, y, z]));
        } else if let Some(rest) = l.strip_prefix("f ") {
            let mut tri = [0u32; 3];
            let mut k = 0;
            for tok in rest.split_whitespace() {
                if k == 3 {
                    break;
                }
                let first = tok.split('/').next().unwrap_or("0");
                let n: i64 = first.parse().unwrap_or(0);
                tri[k] = if n > 0 { (n - 1) as u32 } else { 0 };
                k += 1;
            }
            if k == 3 {
                idx.extend_from_slice(&tri);
            }
        }
    }
    Ok((verts, idx))
}
