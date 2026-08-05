# packaging

What `scripts/package.sh` needs to assemble `Brightfield.app` and the disk image
it ships in. Nothing here is compiled, and no test reads it.

| File | What it is |
|---|---|
| `Info.plist` | The bundle's property list, copied to `Contents/Info.plist`. Its three placeholder values are overwritten per build — see the comment in the file. |
| `Brightfield.icns` | The application icon, copied to `Contents/Resources/`. **Not MIT** — see `LICENSE-BRAND.md`. |
| `Brightfield.svg` | The 1024×1024 master the `.icns` is rasterised from. **Not MIT** — see `LICENSE-BRAND.md`. |
| `make-icns.sh` | Regenerates the `.icns` from the `.svg`. A developer-machine tool; the build does not run it. |
| `LICENSE-BRAND.md` | The carve-out taking the two brand artefacts out of the repository's MIT grant. |

## Where the icon comes from

`Brightfield.svg` is the Meridian prime mark — the path in `meridian_white.svg`
in the design-system repository's `meridian-design/brand/` — drawn white over a
black plate. The plate is a superellipse inscribed in the 824×824 body of the
1024×1024 macOS icon grid, which is the shape the platform's own icons take; a
plain rounded rectangle reads as a foreign application on the Dock beside them.

The mark is not redrawn, recoloured or re-proportioned: the path data is the
master's, and the only transform applied to it is a uniform scale and a
translation to the centre of the canvas. `make-icns.sh` is the whole derivation.

## Two things this bundle deliberately does not carry

**No entitlements plist.** The App Sandbox is initialised from the signature, and
a Developer ID build has no reason to declare the key. Adding one would take file
access away from a program whose whole job is to open the files a specification
names.

**No signature from a real identity, yet.** `scripts/package.sh` signs the bundle
ad-hoc unless `BRIGHTFIELD_SIGN_IDENTITY` names a certificate, and the header of
that script documents the chain that runs when one does.
