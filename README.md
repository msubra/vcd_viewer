# vcd_report

Rust CLI that scans a folder of `.vcd` files and generates a stacked-row HTML waveform report.

## Build

```bash
cargo build --release
```

Binary:

```bash
./target/release/vcd_report
```

## Run

```bash
./target/release/vcd_report /Users/msubra/dev/vcd -o d1.html
```

For huge files, start with conservative limits:

```bash
./target/release/vcd_report /Users/msubra/dev/vcd \
  -o d1.html \
  --max-signals-per-file 1 \
  --max-transitions-per-row 2000
```

Only include useful signal names:

```bash
./target/release/vcd_report /Users/msubra/dev/vcd \
  -o d1.html \
  --signal-regex "clk|dec|dut|op|result" \
  --max-signals-per-file 1 \
  --max-transitions-per-row 2000
```

Include all matching signals per file:

```bash
./target/release/vcd_report /Users/msubra/dev/vcd \
  -o d1.html \
  --signal-regex "clk|dec|dut|op|result" \
  --max-signals-per-file 0 \
  --max-transitions-per-row 2000
```

Search subfolders:

```bash
./target/release/vcd_report /Users/msubra/dev/vcd --recursive -o d1.html
```

## Rendering

Reports are self-contained HTML and render waveforms with built-in canvas code.
They do not need Plotly, a CDN, or a dev server, so they work directly in
`file://` mode.

Use the mouse wheel or trackpad to zoom around the cursor, drag horizontally to
pan, and double-click the waveform to reset the time range.

## Notes

This parser supports common ModelSim VCD exports, including buses exported as separate bit signals:

```text
dec_op_a [7]
dec_op_a [6]
...
dec_op_a [0]
```

Those bits are reconstructed into one bus row named `dec_op_a`.
