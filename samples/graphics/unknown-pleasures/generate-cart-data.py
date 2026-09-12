#!/usr/bin/env python3
"""Keep the 50 displayed CP1919 traces, with all 300 samples per trace."""

from pathlib import Path
import re


def main():
    project = Path(__file__).resolve().parent
    source = (project / "unknown-pleasures-data.inc").read_text().split("=[")[1]
    heights = [int(value, 16) for value in re.findall(r"\$([0-9A-F]{2})", source)]
    if len(heights) != 80 * 300:
        raise SystemExit("expected 80 traces of 300 heights in the source table")
    # Use the same 50-of-80 selection as unknown-pleasures.act, ahead of time
    # so the beginner program only needs to read consecutive data rows.
    selected = []
    selector = 30
    for trace in range(80):
        selector += 50
        if selector >= 80:
            selector -= 80
            selected.extend(heights[trace * 300:(trace + 1) * 300])
    if len(selected) != 15000:
        raise SystemExit("expected 50 traces of 300 heights")
    for index, height in enumerate(selected):
        baseline = 181 - (index // 300) * 3
        if not 0 <= baseline + 4 - height < 192:
            raise SystemExit("pulse height exceeds the cartridge renderer's screen")

    lines = [
        "; CP1919 pulse heights for the beginner cartridge example.",
        "; Same 50 displayed traces as unknown-pleasures.act, selected in advance.",
        "; All 300 heights per trace, with the same height bias of 4.",
        "; See README.md and unknown-pleasures-data.inc for data provenance.",
        "",
        "BYTE ARRAY pulseData(15000)=[",
    ]
    for offset in range(0, len(selected), 15):
        lines.append("  " + " ".join(str(value) for value in selected[offset:offset + 15]))
    lines[-1] += "]"
    (project / "UPDATA.ACT").write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
