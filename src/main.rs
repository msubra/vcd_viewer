use clap::Parser;
use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(
    name = "vcd_report",
    version,
    about = "Generate a stacked waveform HTML report from all VCD files in a folder"
)]
struct Args {
    /// Folder containing .vcd files.
    folder: PathBuf,

    /// Output HTML file.
    #[arg(short, long, default_value = "vcd_folder_waveform_report.html")]
    output: PathBuf,

    /// Search subfolders recursively for .vcd files.
    #[arg(long)]
    recursive: bool,

    /// Report title.
    #[arg(long, default_value = "VCD Folder Waveform Report")]
    title: String,

    /// Only include signal names matching this regex, for example: "clk|dec|result|valid".
    #[arg(long)]
    signal_regex: Option<String>,

    /// Maximum selected signals per VCD file. Default: 1. Use 0 for all matching signals.
    #[arg(long, default_value_t = 1)]
    max_signals_per_file: usize,

    /// Maximum stored transitions per waveform row. Default: 2000. Use 0 for no cap.
    #[arg(long, default_value_t = 2000)]
    max_transitions_per_row: usize,

    /// Prefix to remove from row labels. Pass empty string to disable.
    #[arg(long, default_value = "jun-27-2026-export-simulation-waveform-")]
    strip_prefix: String,

    /// Deprecated. Reports now render with built-in canvas code and do not require Plotly.
    /// Accepted for compatibility with older scripts.
    #[arg(long)]
    plotly_js: Option<PathBuf>,
}

#[derive(Clone, Debug)]
struct SignalDef {
    name: String,
    width: usize,
    bit: Option<usize>,
}

#[derive(Clone, Debug)]
struct ParsedSignal {
    name: String,
    width: usize,
    history: Vec<(u64, String)>,
}

#[derive(Serialize)]
struct ReportData {
    title: String,
    summary: Summary,
    #[serde(rename = "fileStats")]
    file_stats: Vec<FileStat>,
    #[serde(rename = "waveformRows")]
    waveform_rows: Vec<WaveformRow>,
}

#[derive(Serialize)]
struct Summary {
    input_folder: String,
    files_seen: usize,
    files_with_selected_signals: usize,
    waveform_rows: usize,
    max_transitions_per_row: usize,
    max_signals_per_file: usize,
    time_unit: String,
}

#[derive(Serialize)]
struct FileStat {
    file: String,
    timescale: String,
    size_bytes: u64,
    selected_signals: usize,
    total_changes_seen: u64,
    selected_changes_seen: u64,
    stored_changes: u64,
    skipped_due_to_limit: u64,
}

#[derive(Serialize)]
struct WaveformRow {
    instance: String,
    file: String,
    signal: String,
    label: String,
    width: usize,
    timescale: String,
    points: Vec<Point>,
    point_count: usize,
    last_value: String,
    last_hex: String,
    last_decimal: Option<u64>,
}

#[derive(Serialize)]
struct Point {
    time: u64,
    binary: String,
    hex: String,
    decimal: Option<u64>,
}

#[derive(Debug)]
struct Definitions {
    timescale: String,
    symbol_defs: HashMap<String, SignalDef>,
    signal_bits: BTreeMap<String, BTreeSet<usize>>,
    widths: HashMap<String, usize>,
}

#[derive(Debug)]
struct ParseMeta {
    size_bytes: u64,
    total_changes_seen: u64,
    selected_changes_seen: u64,
    stored_changes: u64,
    skipped_due_to_limit: u64,
}

fn main() {
    let args = Args::parse();

    match run(args) {
        Ok(()) => {}
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    }
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    if !args.folder.exists() {
        return Err(format!("Folder does not exist: {}", args.folder.display()).into());
    }

    if !args.folder.is_dir() {
        return Err(format!("Not a folder: {}", args.folder.display()).into());
    }

    let vcd_files = find_vcd_files(&args.folder, args.recursive)?;

    if vcd_files.is_empty() {
        return Err(format!("No .vcd files found in folder: {}", args.folder.display()).into());
    }

    let signal_re = match &args.signal_regex {
        Some(pattern) => Some(Regex::new(pattern)?),
        None => None,
    };

    let mut waveform_rows = Vec::new();
    let mut file_stats = Vec::new();

    for file_path in &vcd_files {
        let file_name = file_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("<unknown>")
            .to_string();

        println!("Signal file {}", file_name);

        let defs = read_definitions(file_path)?;
        let selected_names =
            select_signal_names(&defs, signal_re.as_ref(), args.max_signals_per_file);

        if selected_names.is_empty() {
            println!("  No matching signals selected in {}", file_name);
            continue;
        }

        let (timescale, signals, meta) =
            parse_selected_signals(file_path, &selected_names, args.max_transitions_per_row)?;

        for signal in signals.values() {
            waveform_rows.push(make_waveform_row(
                file_path,
                signal,
                &timescale,
                &args.strip_prefix,
            )?);
        }

        file_stats.push(FileStat {
            file: file_name,
            timescale,
            size_bytes: meta.size_bytes,
            selected_signals: signals.len(),
            total_changes_seen: meta.total_changes_seen,
            selected_changes_seen: meta.selected_changes_seen,
            stored_changes: meta.stored_changes,
            skipped_due_to_limit: meta.skipped_due_to_limit,
        });
    }

    let report = ReportData {
        title: args.title.clone(),
        summary: Summary {
            input_folder: args.folder.canonicalize()?.display().to_string(),
            files_seen: vcd_files.len(),
            files_with_selected_signals: file_stats.len(),
            waveform_rows: waveform_rows.len(),
            max_transitions_per_row: args.max_transitions_per_row,
            max_signals_per_file: args.max_signals_per_file,
            time_unit: "ps".to_string(),
        },
        file_stats,
        waveform_rows,
    };

    generate_html(&report, &args.output, args.plotly_js.as_deref())?;

    println!("Wrote HTML report: {}", args.output.display());
    println!("Waveform rows: {}", report.summary.waveform_rows);

    Ok(())
}

fn find_vcd_files(folder: &Path, recursive: bool) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    if recursive {
        for entry in WalkDir::new(folder).into_iter().filter_map(Result::ok) {
            let path = entry.path();
            if path.is_file() && is_vcd_file(path) {
                files.push(path.to_path_buf());
            }
        }
    } else {
        for entry in fs::read_dir(folder)? {
            let path = entry?.path();
            if path.is_file() && is_vcd_file(&path) {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

fn is_vcd_file(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("vcd"))
        .unwrap_or(false)
}

fn read_definitions(file_path: &Path) -> io::Result<Definitions> {
    let file = fs::File::open(file_path)?;
    let reader = io::BufReader::new(file);

    let var_re = Regex::new(r"^\$var\s+\w+\s+(\d+)\s+(\S+)\s+(.+?)\s+\$end").unwrap();
    let bit_re = Regex::new(r"^(.+?)\s+\[(\d+)\]$").unwrap();
    let vector_re = Regex::new(r"^(.+?)\s*\[(\d+)\s*:\s*(\d+)\]$").unwrap();
    let timescale_inline_re = Regex::new(r"^\$timescale\s+(.+?)\s+\$end").unwrap();

    let mut timescale = "unknown".to_string();
    let mut in_timescale_multiline = false;

    let mut symbol_defs: HashMap<String, SignalDef> = HashMap::new();
    let mut signal_bits: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    let mut widths: HashMap<String, usize> = HashMap::new();

    for line_result in reader.lines() {
        let raw = line_result?;
        let line = raw.trim();

        if in_timescale_multiline {
            if line == "$end" {
                in_timescale_multiline = false;
            } else if !line.is_empty() {
                timescale = line.to_string();
            }
            continue;
        }

        if line.starts_with("$timescale") {
            if let Some(caps) = timescale_inline_re.captures(line) {
                timescale = caps[1].trim().to_string();
            } else {
                in_timescale_multiline = true;
            }
            continue;
        }

        if let Some(caps) = var_re.captures(line) {
            let width: usize = caps[1].parse().unwrap_or(1);
            let symbol = caps[2].to_string();
            let raw_name = caps[3].trim().to_string();

            if let Some(bit_caps) = bit_re.captures(&raw_name) {
                if width == 1 {
                    let base = bit_caps[1].trim().to_string();
                    let bit: usize = bit_caps[2].parse().unwrap_or(0);

                    symbol_defs.insert(
                        symbol,
                        SignalDef {
                            name: base.clone(),
                            width: 1,
                            bit: Some(bit),
                        },
                    );

                    signal_bits.entry(base.clone()).or_default().insert(bit);
                    let entry = widths.entry(base).or_insert(0);
                    *entry = (*entry).max(bit + 1);
                    continue;
                }
            }

            if let Some(vec_caps) = vector_re.captures(&raw_name) {
                let base = vec_caps[1].trim().to_string();
                let hi: usize = vec_caps[2].parse().unwrap_or(0);
                let lo: usize = vec_caps[3].parse().unwrap_or(0);

                symbol_defs.insert(
                    symbol,
                    SignalDef {
                        name: base.clone(),
                        width,
                        bit: None,
                    },
                );

                let min_bit = hi.min(lo);
                let max_bit = hi.max(lo);
                for bit in min_bit..=max_bit {
                    signal_bits.entry(base.clone()).or_default().insert(bit);
                }

                let entry = widths.entry(base).or_insert(0);
                *entry = (*entry).max(width);
            } else {
                symbol_defs.insert(
                    symbol,
                    SignalDef {
                        name: raw_name.clone(),
                        width,
                        bit: None,
                    },
                );

                signal_bits.entry(raw_name.clone()).or_default();
                let entry = widths.entry(raw_name).or_insert(0);
                *entry = (*entry).max(width);
            }

            continue;
        }

        if line.starts_with("$enddefinitions") {
            break;
        }
    }

    Ok(Definitions {
        timescale,
        symbol_defs,
        signal_bits,
        widths,
    })
}

fn select_signal_names(
    defs: &Definitions,
    signal_re: Option<&Regex>,
    max_signals_per_file: usize,
) -> Vec<String> {
    let mut names: Vec<String> = defs.signal_bits.keys().cloned().collect();

    if let Some(re) = signal_re {
        names.retain(|name| re.is_match(name));
    }

    if max_signals_per_file > 0 && names.len() > max_signals_per_file {
        names.truncate(max_signals_per_file);
    }

    names
}

fn parse_selected_signals(
    file_path: &Path,
    selected_names: &[String],
    max_transitions_per_row: usize,
) -> Result<(String, BTreeMap<String, ParsedSignal>, ParseMeta), Box<dyn std::error::Error>> {
    let defs = read_definitions(file_path)?;

    let selected: HashSet<String> = selected_names.iter().cloned().collect();

    let selected_symbols: HashSet<String> = defs
        .symbol_defs
        .iter()
        .filter_map(|(symbol, sigdef)| {
            if selected.contains(&sigdef.name) {
                Some(symbol.clone())
            } else {
                None
            }
        })
        .collect();

    let mut signal_state: HashMap<(String, Option<usize>), String> = HashMap::new();
    let mut histories: BTreeMap<String, Vec<(u64, String)>> = BTreeMap::new();

    for name in selected_names {
        let bits = defs.signal_bits.get(name).cloned().unwrap_or_default();
        let width = defs.widths.get(name).copied().unwrap_or(1).max(1);

        let initial = if bits.is_empty() {
            signal_state.insert((name.clone(), None), "x".repeat(width));
            "x".repeat(width)
        } else {
            for bit in &bits {
                signal_state.insert((name.clone(), Some(*bit)), "x".to_string());
            }

            bits.iter().rev().map(|_| "x").collect::<Vec<_>>().join("")
        };

        histories.insert(name.clone(), vec![(0, initial)]);
    }

    let mut last: HashMap<String, String> = histories
        .iter()
        .filter_map(|(name, hist)| hist.first().map(|(_, value)| (name.clone(), value.clone())))
        .collect();

    let file = fs::File::open(file_path)?;
    let reader = io::BufReader::new(file);

    let mut current_time: u64 = 0;
    let mut in_defs = true;

    let mut total_changes_seen = 0u64;
    let mut selected_changes_seen = 0u64;
    let mut stored_changes = 0u64;
    let mut skipped_due_to_limit = 0u64;

    for line_result in reader.lines() {
        let raw = line_result?;
        let line = raw.trim();

        if in_defs {
            if line.starts_with("$enddefinitions") {
                in_defs = false;
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix('#') {
            if let Ok(t) = rest.parse::<u64>() {
                current_time = t;
            }
            continue;
        }

        let parsed_change = parse_value_change(line);
        let Some((symbol, value)) = parsed_change else {
            continue;
        };

        if !defs.symbol_defs.contains_key(&symbol) {
            continue;
        }

        total_changes_seen += 1;

        if !selected_symbols.contains(&symbol) {
            continue;
        }

        selected_changes_seen += 1;

        let sigdef = defs.symbol_defs.get(&symbol).unwrap();
        let name = sigdef.name.clone();

        match sigdef.bit {
            None => {
                let width = defs
                    .widths
                    .get(&name)
                    .copied()
                    .unwrap_or(sigdef.width)
                    .max(1);
                let mut v = value;
                if v.len() < width && v.chars().all(|c| matches!(c, '0' | '1' | 'x' | 'z')) {
                    v = format!("{}{}", "0".repeat(width - v.len()), v);
                }
                signal_state.insert((name.clone(), None), v);
            }
            Some(bit) => {
                signal_state.insert((name.clone(), Some(bit)), value);
            }
        }

        let new_value = current_value(&name, &defs, &signal_state);

        if last.get(&name) != Some(&new_value) {
            let hist = histories.get_mut(&name).unwrap();

            if max_transitions_per_row == 0 || hist.len() < max_transitions_per_row {
                hist.push((current_time, new_value.clone()));
                stored_changes += 1;
            } else if let Some(last_point) = hist.last_mut() {
                *last_point = (current_time, new_value.clone());
                skipped_due_to_limit += 1;
            }

            last.insert(name, new_value);
        }
    }

    let mut signals = BTreeMap::new();

    for name in selected_names {
        if let Some(history) = histories.remove(name) {
            let width = defs
                .widths
                .get(name)
                .copied()
                .unwrap_or_else(|| history.last().map(|(_, v)| v.len()).unwrap_or(1))
                .max(1);

            signals.insert(
                name.clone(),
                ParsedSignal {
                    name: name.clone(),
                    width,
                    history,
                },
            );
        }
    }

    let size_bytes = fs::metadata(file_path)?.len();

    Ok((
        defs.timescale,
        signals,
        ParseMeta {
            size_bytes,
            total_changes_seen,
            selected_changes_seen,
            stored_changes,
            skipped_due_to_limit,
        },
    ))
}

fn parse_value_change(line: &str) -> Option<(String, String)> {
    let mut chars = line.chars();
    let first = chars.next()?;

    if matches!(first, '0' | '1' | 'x' | 'X' | 'z' | 'Z') {
        let symbol: String = chars.collect();
        if symbol.is_empty() {
            None
        } else {
            Some((symbol, first.to_ascii_lowercase().to_string()))
        }
    } else if first == 'b' || first == 'B' {
        let mut parts = line.split_whitespace();
        let value_part = parts.next()?;
        let symbol = parts.next()?;

        if parts.next().is_some() {
            return None;
        }

        let value = value_part[1..].to_ascii_lowercase();
        Some((symbol.to_string(), value))
    } else {
        None
    }
}

fn current_value(
    name: &str,
    defs: &Definitions,
    signal_state: &HashMap<(String, Option<usize>), String>,
) -> String {
    let bits = defs.signal_bits.get(name).cloned().unwrap_or_default();

    if bits.is_empty() {
        return signal_state
            .get(&(name.to_string(), None))
            .cloned()
            .unwrap_or_else(|| "x".to_string());
    }

    bits.iter()
        .rev()
        .map(|bit| {
            signal_state
                .get(&(name.to_string(), Some(*bit)))
                .cloned()
                .unwrap_or_else(|| "x".to_string())
        })
        .collect::<Vec<_>>()
        .join("")
}

fn value_to_int(value: &str) -> Option<u64> {
    let lower = value.to_ascii_lowercase();

    if lower.contains('x') || lower.contains('z') {
        return None;
    }

    u64::from_str_radix(&lower, 2).ok()
}

fn value_to_hex(value: &str) -> String {
    match value_to_int(value) {
        None => "unknown".to_string(),
        Some(n) => {
            let digits = ((value.len() + 3) / 4).max(1);
            format!("0x{:0width$X}", n, width = digits)
        }
    }
}

fn label_from_filename(path: &Path, strip_prefix: &str) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let stripped = if !strip_prefix.is_empty() && stem.starts_with(strip_prefix) {
        stem[strip_prefix.len()..].to_string()
    } else {
        stem
    };

    stripped
        .trim_matches(|c| c == '_' || c == '-' || c == ' ')
        .to_string()
}

fn make_waveform_row(
    file_path: &Path,
    signal: &ParsedSignal,
    timescale: &str,
    strip_prefix: &str,
) -> Result<WaveformRow, Box<dyn std::error::Error>> {
    let points: Vec<Point> = signal
        .history
        .iter()
        .map(|(time, value)| Point {
            time: *time,
            binary: value.clone(),
            hex: value_to_hex(value),
            decimal: value_to_int(value),
        })
        .collect();

    let instance = label_from_filename(file_path, strip_prefix);
    let file = file_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("<unknown>")
        .to_string();

    let last_value = signal
        .history
        .last()
        .map(|(_, value)| value.clone())
        .unwrap_or_default();

    let point_count = points.len();

    Ok(WaveformRow {
        instance: instance.clone(),
        file,
        signal: signal.name.clone(),
        label: format!("{} :: {}", instance, signal.name),
        width: signal.width,
        timescale: timescale.to_string(),
        points,
        point_count,
        last_hex: value_to_hex(&last_value),
        last_decimal: value_to_int(&last_value),
        last_value,
    })
}

fn generate_html(
    report: &ReportData,
    output_path: &Path,
    _plotly_js_path: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let payload_json = serde_json::to_string(report)?;

    let title_escaped = html_escape::encode_text(&report.title).to_string();

    let template = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>__TITLE__</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>
:root {
  --bg: #0f172a;
  --panel: #111827;
  --text: #e5e7eb;
  --muted: #9ca3af;
  --line: #374151;
}
html, body {
  margin: 0;
  min-height: 100%;
  background: var(--bg);
  color: var(--text);
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}
main {
  padding: 14px;
}
.card {
  background: var(--panel);
  border: 1px solid var(--line);
  border-radius: 14px;
  padding: 14px;
}
.controls {
  display: grid;
  grid-template-columns: 1fr 1fr 160px;
  gap: 12px;
  margin-bottom: 8px;
}
@media (max-width: 900px) {
  .controls {
    grid-template-columns: 1fr;
  }
}
select, input {
  width: 100%;
  box-sizing: border-box;
  border-radius: 10px;
  border: 1px solid var(--line);
  background: #0b1220;
  color: var(--text);
  padding: 10px;
}
label {
  color: var(--muted);
  font-size: 13px;
  display: block;
  margin-bottom: 5px;
}
.waveform-shell {
  width: 100%;
  position: relative;
  overflow: hidden;
  border: 1px solid var(--line);
  border-radius: 10px;
  background: #0b1220;
}
.waveform-wrap {
  position: relative;
  min-height: 560px;
}
#waveform {
  display: block;
  width: 100%;
  height: 560px;
  cursor: crosshair;
}
.empty-state {
  min-height: 560px;
  display: grid;
  place-items: center;
  color: var(--muted);
}
.tooltip {
  position: fixed;
  z-index: 10;
  display: none;
  max-width: 360px;
  padding: 8px 10px;
  border: 1px solid var(--line);
  border-radius: 8px;
  background: #020617;
  color: var(--text);
  font-size: 12px;
  line-height: 1.45;
  pointer-events: none;
  box-shadow: 0 10px 30px rgb(0 0 0 / 0.35);
}
.hint {
  color: var(--muted);
  font-size: 12px;
  margin-top: 8px;
}
</style>
</head>
<body>
<main>
  <section class="card">
    <div class="controls">
      <div>
        <label for="instanceFilter">File / instance</label>
        <select id="instanceFilter"></select>
      </div>
      <div>
        <label for="textFilter">Filter rows</label>
        <input id="textFilter" placeholder="dec_a, result, op, valid">
      </div>
      <div>
        <label for="maxRows">Max rows</label>
        <input id="maxRows" type="number" value="80" min="1">
      </div>
    </div>
    <div class="waveform-shell">
      <div class="waveform-wrap">
        <canvas id="waveform"></canvas>
        <div id="emptyState" class="empty-state" hidden>No matching waveform rows.</div>
      </div>
    </div>
    <div class="hint">Wheel or trackpad scroll zooms around the cursor. Drag horizontally to pan. Double-click resets the view.</div>
  </section>
</main>

<div id="tooltip" class="tooltip"></div>

<script>
const DATA = __PAYLOAD__;
const canvas = document.getElementById("waveform");
const ctx = canvas.getContext("2d", { alpha: false });
const tooltip = document.getElementById("tooltip");
const emptyState = document.getElementById("emptyState");

const view = {
  minTime: 0,
  maxTime: 1,
  fullMinTime: 0,
  fullMaxTime: 1,
  rows: [],
  hover: null,
  dragging: false,
  dragStartX: 0,
  dragStartMinTime: 0,
  dragStartMaxTime: 1,
};

const metrics = {
  left: 360,
  right: 24,
  top: 24,
  bottom: 54,
  rowHeight: 42,
  minHeight: 560,
};

function escapeAttr(s) {
  return String(s)
    .replaceAll("&", "&amp;")
    .replaceAll('"', "&quot;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}

function unique(values) {
  return [...new Set(values)].sort();
}

function formatSimulationTime(ps) {
  if (ps === null || ps === undefined || Number.isNaN(Number(ps))) return "";
  const v = Number(ps);
  const av = Math.abs(v);

  if (av >= 1_000_000_000) return `${(v / 1_000_000_000).toFixed(3).replace(/\.0+$/, "")} ms`;
  if (av >= 1_000_000) return `${(v / 1_000_000).toFixed(3).replace(/\.0+$/, "")} us`;
  if (av >= 1_000) return `${(v / 1_000).toFixed(3).replace(/\.0+$/, "")} ns`;
  return `${v} ps`;
}

function formatValue(point) {
  if (!point) return "";
  if (point.hex && point.hex !== "unknown") return `${point.hex} (${point.decimal})`;
  return point.binary || "unknown";
}

function shortRowLabel(row) {
  const signal = row.signal || "";
  const instance = row.instance || row.file || "";
  const maxInstance = 34;
  const clipped = instance.length > maxInstance ? "..." + instance.slice(instance.length - maxInstance) : instance;
  return `${clipped} :: ${signal}`;
}

function fillInstanceFilter() {
  const instances = unique(DATA.waveformRows.map(row => row.instance));

  document.getElementById("instanceFilter").innerHTML =
    `<option value="__ALL__">All files</option>` +
    instances.map(instance => `<option value="${escapeAttr(instance)}">${instance}</option>`).join("");
}

function filteredRows() {
  const instance = document.getElementById("instanceFilter").value;
  const text = document.getElementById("textFilter").value.toLowerCase().trim();
  const maxRows = Math.max(1, Number(document.getElementById("maxRows").value || 80));

  return DATA.waveformRows
    .filter(row => instance === "__ALL__" || row.instance === instance)
    .filter(row => {
      if (!text) return true;
      return row.label.toLowerCase().includes(text) ||
             row.file.toLowerCase().includes(text) ||
             row.signal.toLowerCase().includes(text);
    })
    .slice(0, maxRows);
}

function visibleTimeRange(rows) {
  let minTime = Infinity;
  let maxTime = -Infinity;

  rows.forEach(row => {
    row.points.forEach(point => {
      minTime = Math.min(minTime, point.time);
      maxTime = Math.max(maxTime, point.time);
    });
  });

  if (!Number.isFinite(minTime) || !Number.isFinite(maxTime)) {
    return [0, 1];
  }

  if (minTime === maxTime) {
    return [Math.max(0, minTime - 1), maxTime + 1];
  }

  return [minTime, maxTime];
}

function syncRows(resetRange) {
  const rows = filteredRows();
  view.rows = rows;

  if (!rows.length) {
    emptyState.hidden = false;
    canvas.hidden = true;
    tooltip.style.display = "none";
    draw();
    return;
  }

  emptyState.hidden = true;
  canvas.hidden = false;

  const [minTime, maxTime] = visibleTimeRange(rows);
  view.fullMinTime = minTime;
  view.fullMaxTime = maxTime;

  if (resetRange || view.minTime < minTime || view.maxTime > maxTime || view.minTime >= view.maxTime) {
    view.minTime = minTime;
    view.maxTime = maxTime;
  }

  draw();
}

function resizeCanvas() {
  const dpr = window.devicePixelRatio || 1;
  const cssWidth = Math.max(640, canvas.parentElement.clientWidth);
  const cssHeight = Math.max(metrics.minHeight, view.rows.length * metrics.rowHeight + metrics.top + metrics.bottom);

  canvas.style.height = `${cssHeight}px`;
  canvas.width = Math.floor(cssWidth * dpr);
  canvas.height = Math.floor(cssHeight * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  draw();
}

function plotWidth() {
  return Math.max(1, canvas.clientWidth - metrics.left - metrics.right);
}

function xForTime(time) {
  const span = Math.max(1, view.maxTime - view.minTime);
  return metrics.left + ((time - view.minTime) / span) * plotWidth();
}

function timeForX(x) {
  const ratio = (x - metrics.left) / plotWidth();
  return view.minTime + ratio * (view.maxTime - view.minTime);
}

function rowCenter(index) {
  return metrics.top + index * metrics.rowHeight + metrics.rowHeight / 2;
}

function valueY(row, point, center) {
  if (row.width === 1 && point.decimal !== null && point.decimal !== undefined) {
    return center + (point.decimal ? -12 : 12);
  }

  return center;
}

function clampRange(minTime, maxTime) {
  const fullSpan = Math.max(1, view.fullMaxTime - view.fullMinTime);
  let span = Math.max(1, maxTime - minTime);

  if (span > fullSpan) {
    span = fullSpan;
    minTime = view.fullMinTime;
    maxTime = view.fullMaxTime;
  }

  if (minTime < view.fullMinTime) {
    maxTime += view.fullMinTime - minTime;
    minTime = view.fullMinTime;
  }

  if (maxTime > view.fullMaxTime) {
    minTime -= maxTime - view.fullMaxTime;
    maxTime = view.fullMaxTime;
  }

  view.minTime = Math.max(view.fullMinTime, minTime);
  view.maxTime = Math.min(view.fullMaxTime, maxTime);
}

function drawGrid(width, height) {
  ctx.fillStyle = "#0b1220";
  ctx.fillRect(0, 0, width, height);

  ctx.strokeStyle = "#1f2937";
  ctx.lineWidth = 1;
  ctx.beginPath();
  view.rows.forEach((_, index) => {
    const y = rowCenter(index);
    ctx.moveTo(metrics.left, y);
    ctx.lineTo(width - metrics.right, y);
  });
  ctx.stroke();

  ctx.strokeStyle = "#374151";
  ctx.fillStyle = "#9ca3af";
  ctx.font = "12px -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif";
  ctx.textAlign = "center";
  ctx.textBaseline = "top";

  const ticks = 8;
  for (let i = 0; i <= ticks; i++) {
    const t = view.minTime + ((view.maxTime - view.minTime) * i / ticks);
    const x = xForTime(t);
    ctx.beginPath();
    ctx.moveTo(x, metrics.top);
    ctx.lineTo(x, height - metrics.bottom + 10);
    ctx.stroke();
    ctx.fillText(formatSimulationTime(Math.round(t)), x, height - metrics.bottom + 18);
  }
}

function drawLabels() {
  ctx.textAlign = "right";
  ctx.textBaseline = "middle";
  ctx.font = "12px -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif";

  view.rows.forEach((row, index) => {
    ctx.fillStyle = "#e5e7eb";
    ctx.fillText(shortRowLabel(row), metrics.left - 14, rowCenter(index), metrics.left - 24);
  });
}

function visiblePoints(row) {
  const points = row.points;
  if (points.length <= 2) return points;

  let start = 0;
  while (start < points.length - 1 && points[start + 1].time < view.minTime) {
    start++;
  }

  let end = start;
  while (end < points.length && points[end].time <= view.maxTime) {
    end++;
  }

  if (end < points.length) end++;
  return points.slice(start, end);
}

function drawRows() {
  ctx.strokeStyle = "#60a5fa";
  ctx.fillStyle = "#93c5fd";
  ctx.lineWidth = 2;

  view.rows.forEach((row, index) => {
    const points = visiblePoints(row);
    if (!points.length) return;

    const center = rowCenter(index);
    ctx.beginPath();

    points.forEach((point, pointIndex) => {
      const x = xForTime(point.time);
      const y = valueY(row, point, center);

      if (pointIndex === 0) {
        ctx.moveTo(x, y);
      } else {
        const prev = points[pointIndex - 1];
        const prevY = valueY(row, prev, center);
        ctx.lineTo(x, prevY);
        ctx.lineTo(x, y);
      }
    });

    ctx.stroke();

    points.forEach(point => {
      const x = xForTime(point.time);
      if (x < metrics.left - 8 || x > canvas.clientWidth - metrics.right + 8) return;

      const y = valueY(row, point, center);
      ctx.beginPath();
      ctx.arc(x, y, row.width === 1 ? 3 : 4, 0, Math.PI * 2);
      ctx.fill();
    });
  });
}

function drawHover(height) {
  const hover = view.hover;
  if (!hover) return;

  const x = xForTime(hover.point.time);
  const y = rowCenter(hover.rowIndex);

  ctx.strokeStyle = "#f59e0b";
  ctx.fillStyle = "#fbbf24";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(x, metrics.top);
  ctx.lineTo(x, height - metrics.bottom);
  ctx.stroke();

  ctx.beginPath();
  ctx.arc(x, valueY(hover.row, hover.point, y), 5, 0, Math.PI * 2);
  ctx.fill();
}

function draw() {
  if (!ctx || !canvas.width || !canvas.height) return;

  const width = canvas.clientWidth;
  const height = canvas.clientHeight;
  drawGrid(width, height);
  drawLabels();
  drawRows();
  drawHover(height);
}

function nearestPoint(mouseX, mouseY) {
  const rowIndex = Math.floor((mouseY - metrics.top) / metrics.rowHeight);
  if (rowIndex < 0 || rowIndex >= view.rows.length) return null;

  const row = view.rows[rowIndex];
  const time = timeForX(mouseX);
  let best = null;
  let bestDistance = Infinity;

  visiblePoints(row).forEach(point => {
    const dx = Math.abs(point.time - time);
    if (dx < bestDistance) {
      best = point;
      bestDistance = dx;
    }
  });

  if (!best) return null;

  const pointX = xForTime(best.time);
  const pointY = valueY(row, best, rowCenter(rowIndex));
  const pixelDistance = Math.hypot(pointX - mouseX, pointY - mouseY);

  if (pixelDistance > 42) return null;
  return { row, rowIndex, point: best };
}

function updateTooltip(event, hover) {
  if (!hover) {
    tooltip.style.display = "none";
    return;
  }

  tooltip.innerHTML =
    `<strong>${escapeAttr(hover.row.signal)}</strong><br>` +
    `file=${escapeAttr(hover.row.file)}<br>` +
    `time=${escapeAttr(formatSimulationTime(hover.point.time))}<br>` +
    `binary=${escapeAttr(hover.point.binary)}<br>` +
    `value=${escapeAttr(formatValue(hover.point))}`;
  tooltip.style.left = `${event.clientX + 14}px`;
  tooltip.style.top = `${event.clientY + 14}px`;
  tooltip.style.display = "block";
}

function onPointerMove(event) {
  const rect = canvas.getBoundingClientRect();
  const x = event.clientX - rect.left;
  const y = event.clientY - rect.top;

  if (view.dragging) {
    const dx = x - view.dragStartX;
    const dt = -(dx / plotWidth()) * (view.dragStartMaxTime - view.dragStartMinTime);
    clampRange(view.dragStartMinTime + dt, view.dragStartMaxTime + dt);
    draw();
    return;
  }

  view.hover = nearestPoint(x, y);
  updateTooltip(event, view.hover);
  draw();
}

function onWheel(event) {
  if (!view.rows.length) return;
  event.preventDefault();

  const rect = canvas.getBoundingClientRect();
  const x = event.clientX - rect.left;
  const anchor = timeForX(x);
  const currentSpan = view.maxTime - view.minTime;
  const zoom = Math.exp(Math.sign(event.deltaY) * 0.2);
  const nextSpan = Math.max(1, currentSpan * zoom);
  const ratio = (anchor - view.minTime) / currentSpan;
  const minTime = anchor - nextSpan * ratio;
  const maxTime = minTime + nextSpan;

  clampRange(minTime, maxTime);
  draw();
}

function renderPlot(resetRange = true) {
  syncRows(resetRange);
  resizeCanvas();
}

function init() {
  fillInstanceFilter();
  renderPlot();

  document.getElementById("instanceFilter").addEventListener("change", renderPlot);
  document.getElementById("textFilter").addEventListener("input", renderPlot);
  document.getElementById("maxRows").addEventListener("change", renderPlot);
  window.addEventListener("resize", resizeCanvas);
  canvas.addEventListener("pointermove", onPointerMove);
  canvas.addEventListener("pointerleave", () => {
    view.hover = null;
    tooltip.style.display = "none";
    draw();
  });
  canvas.addEventListener("pointerdown", event => {
    const rect = canvas.getBoundingClientRect();
    view.dragging = true;
    view.dragStartX = event.clientX - rect.left;
    view.dragStartMinTime = view.minTime;
    view.dragStartMaxTime = view.maxTime;
    canvas.setPointerCapture(event.pointerId);
  });
  canvas.addEventListener("pointerup", event => {
    view.dragging = false;
    canvas.releasePointerCapture(event.pointerId);
  });
  canvas.addEventListener("dblclick", () => {
    view.minTime = view.fullMinTime;
    view.maxTime = view.fullMaxTime;
    draw();
  });
  canvas.addEventListener("wheel", onWheel, { passive: false });
}

init();
</script>
</body>
</html>
"##;

    let html = template
        .replace("__TITLE__", &title_escaped)
        .replace("__PAYLOAD__", &payload_json);

    fs::write(output_path, html)?;
    Ok(())
}
