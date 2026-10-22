# Clean-room policy

This project reimplements user-visible behaviour. It does not copy software.

## What is allowed

- Observing and describing behaviour: screens, flows, decisions, timings that a
  user can see, and the results of published documentation.
- Reading public documentation, reviews, help pages, and store listings.
- Writing new code, new text, new icons, and new sounds.
- Studying general platform APIs (X11, Wayland, D-Bus, GTK, WebExtensions).

## What is not allowed

- Copying, translating, or porting source code, smali, decompiled Java, or
  bytecode from the reference app or any commercial product.
- Copying resources: layouts, drawables, icons, fonts, sounds, themes, or
  resource strings.
- Copying asset bundles, database files, catalog data, or domain lists that ship
  with the reference app.
- Committing APKs, native libraries, decompiled trees, or extracted assets.
- Using the reference's trademarks, package identifiers, or branding in a way
  that suggests affiliation.

## Provenance of the analysis

`docs/parity/` describes behaviour derived from a locally held build of Stay
Focused 13.0.2. The analysis was static; the app was never executed. The APK is
not distributed with this repository and must never be committed.

The documentation records **facts about behaviour** (what blocks, when, with
which states). It deliberately omits code, class structure, and resource
contents. Where an implementation detail was unknown, the docs say so.

## Contributor attestation

By contributing you attest that:

1. you did not copy code, resources, or text from a proprietary product;
2. your contribution is your original work or is compatibly licensed;
3. you have the right to submit it under Apache-2.0;
4. you did not use decompiled output as a source file.

If you believe a contribution violates this policy, open a private security
report or contact the maintainers. The maintainers will remove the material
first and investigate second.

## Naming and compatibility

StayProductive is an independent project. References to "Stay Focused" describe
the compatibility target only. Do not use the reference's name, icon, or
screenshots in marketing material; comparisons may use factual descriptions.

## No warranty of parity

Behavioural parity is a goal, not a guarantee. Android-specific capabilities
(uninstall prevention, native-app surface filtering, device admin) have no safe
Linux equivalent and are documented as gaps rather than approximated with
unsafe mechanisms.
