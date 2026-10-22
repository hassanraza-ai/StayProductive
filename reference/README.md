# Local reference material

This directory is **git-ignored** (except this README). It is where heavy,
third-party artifacts live on a developer machine so an agent can consult them
without ever committing them.

Nothing here is required to build StayProductive. The specs in `docs/` are
self-contained.

## What to link

`scripts/link-reference.sh` creates symlinks to a local analysis workspace:

| Link | Target (default) | Why |
| --- | --- | --- |
| `reference/apk/` | `~/rea-targets/*.apk` | the analysed build, for spot checks |
| `reference/decompiled/` | `~/rea-targets/decompiled/` | readable Java reference (never copy from it) |
| `reference/smali-project/` | `~/rea-targets/build/stayfocused-smali/` | decoded resources/manifest for UI inventory |
| `reference/native/` | `~/rea-targets/native/` | native libraries and notes, analysis only |
| `reference/reports/` | `~/rea-targets/REPORT.md`, `FEATURES.md` | full analysis, including material not published here |

## Rules

1. **Read, don't copy.** Use the reference to answer questions the specs leave
   open; never paste code, resources, strings, or assets into a tracked file.
2. **Never commit.** `reference/*` is ignored. If a tool creates files here, they
   stay here.
3. **No redistribution.** The APK and decompiled output are third-party
   copyrighted material. Do not attach them to issues, PRs, or releases.
4. **Cite behaviour, not files.** Specs cite section IDs (`block §3`), not
   decompiled file paths.

## If you do not have the artifacts

That is expected for contributors. Work from `docs/`; open a spec question if a
behaviour is unclear. Maintainers who hold the reference can answer with a
behavioural description, not a code excerpt.
