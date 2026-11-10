# Release records

`ledger.json` is the current public machine-readable index of every GitHub release from
`v3.0.2` onward. It records the tag, publication time, prerelease flag, release URL, and each
asset's published size and SHA-256.

Regenerate it after publishing a release:

```powershell
python scripts/rebuild_release_ledger.py
```

The older per-tag `v*.json` files are retained as historical records from the former release
orchestrator. New releases use the single ledger.
