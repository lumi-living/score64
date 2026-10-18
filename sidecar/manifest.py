"""The sidecar's output contract (ADR-009): files plus a versioned `manifest.json` that score64 validates.

Schema 1:
    {
      "schema": 1,
      "tool": "<script name>", "tool_version": <int>,
      "python": "3.11.x", "packages": {"<distribution>": "<version>", ...},
      "params": {...},
      "inputs":  [{"path": "<as given>", "bytes": n, "blake3": "<hex>"}, ...],
      "outputs": [{"path": "<relative to the output directory>", "bytes": n, "blake3": "<hex>"}, ...]
    }

blake3 is the hash of Score64's content-addressed cache (ADR-004), so sidecar results can go straight into it.
"""

import importlib.metadata
import json
import pathlib
import platform

import blake3

SCHEMA = 1


def file_entry(path: pathlib.Path, shown: str) -> dict:
    data = path.read_bytes()
    return {"path": shown, "bytes": len(data), "blake3": blake3.blake3(data).hexdigest()}


def write(out_dir: pathlib.Path, tool: str, tool_version: int, packages, params: dict, inputs, outputs) -> dict:
    """Write manifest.json into out_dir. inputs: paths; outputs: file names inside out_dir."""
    manifest = {
        "schema": SCHEMA,
        "tool": tool,
        "tool_version": tool_version,
        "python": platform.python_version(),
        "packages": {name: importlib.metadata.version(name) for name in packages},
        "params": params,
        "inputs": [file_entry(pathlib.Path(p), str(p)) for p in inputs],
        "outputs": [file_entry(out_dir / name, name) for name in outputs],
    }
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=1))
    return manifest
