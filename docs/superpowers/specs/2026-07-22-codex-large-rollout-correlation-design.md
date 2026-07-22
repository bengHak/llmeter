# Codex Large Rollout Correlation Design

## Problem

The live collector bootstraps only the last 4 MiB of a session file. Large Codex rollout files therefore omit the leading `session_meta` record. The cursor then uses the entire rollout file stem as the session ID, which cannot be matched to the running Codex process. The live-process filter removes the uncorrelated native metrics row and leaves an empty `process-*` row.

## Design

For Codex rollout files, derive the fallback session ID from the trailing UUID already present in the native filename. Validate it with the existing `uuid` dependency. Keep the current file-stem fallback for malformed or nonstandard filenames and for every other tool.

No extra file read or larger bootstrap window is needed. If `session_meta` is present, the Codex adapter continues to use it as the authoritative ID.

## Verification

Add a cursor test whose rollout is larger than its bootstrap window, so the initial `session_meta` is skipped. The tail event must still use the filename UUID. Then verify the running Codex processes correlate to native sessions and expose numeric NOW/AVG values.
