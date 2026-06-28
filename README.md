# vcd_report

Rust CLI that scans a folder of `.vcd` files and generates a stacked-row HTML waveform report.

## Build

```bash
make build
```

Binary:

```bash
./target/release/vcd_report
```

## Run

```bash
make run
```

By default, `make run` reads `.vcd` files from `data` and writes `d1.html`.
Override inputs and limits with make variables:

```bash
make run FOLDER=/path/to/vcds OUTPUT=report.html
```

For huge files, start with conservative limits:

```bash
make run FOLDER=/path/to/vcds OUTPUT=report.html MAX_SIGNALS=1 MAX_TRANSITIONS=2000
```

Only include useful signal names:

```bash
make run FOLDER=/path/to/vcds SIGNAL_REGEX='clk|dec|dut|op|result' MAX_SIGNALS=1 MAX_TRANSITIONS=2000
```

Include all matching signals per file:

```bash
make run FOLDER=/path/to/vcds SIGNAL_REGEX='clk|dec|dut|op|result' MAX_SIGNALS=0 MAX_TRANSITIONS=2000
```

Search subfolders:

```bash
make run FOLDER=/path/to/vcds RECURSIVE=1 OUTPUT=report.html
```

## Rendering

Reports render waveform rows with Plotly. By default, the generated HTML loads
Plotly from a CDN:

```html
<script src="https://cdn.plot.ly/plotly-2.35.2.min.js"></script>
```

For local `file://` viewing without internet, download `plotly.min.js` and run
the binary with `--plotly-js /path/to/plotly.min.js`.

Plotly provides hover labels, zoom, pan, and a time-range slider in the report.

## Notes

This parser supports common ModelSim VCD exports, including buses exported as separate bit signals:

```text
dec_op_a [7]
dec_op_a [6]
...
dec_op_a [0]
```

Those bits are reconstructed into one bus row named `dec_op_a`.
