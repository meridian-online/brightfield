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
| `Brightfield.entitlements` | The one entitlement the app and the signed tarball binary are signed with. See below. |

## Where the icon comes from

`Brightfield.svg` is the Meridian prime mark — the path in `meridian_white.svg`
in the design-system repository's `meridian-design/brand/` — drawn white over a
black plate. The plate is a superellipse inscribed in the 824×824 body of the
1024×1024 macOS icon grid, which is the shape the platform's own icons take; a
plain rounded rectangle reads as a foreign application on the Dock beside them.

The mark is not redrawn, recoloured or re-proportioned: the path data is the
master's, and the only transform applied to it is a uniform scale and a
translation to the centre of the canvas. `make-icns.sh` is the whole derivation.

## The entitlements file

`Brightfield.entitlements` carries one key, `com.apple.security.cs.disable-library-validation`, and `scripts/package.sh` signs the app with it, ad-hoc or not, and the tarball's binary when an identity signs it. Under the hardened runtime a process loads only libraries signed by its own team, and Brightfield loads DuckDB extensions that DuckDB's repository serves at run time (spatial, excel, sqlite and others), which Meridian's team did not sign. DuckDB's own command-line shell carries the same entitlement for the same reason. The bundled FineType extension does not need it in a signed app, because its library is signed with the app's identity. Any further entitlement is added only with a measured failure that shows why.

## Two things this bundle deliberately does not carry

**No App Sandbox.** The sandbox is initialised from the signature's entitlements, and a Developer ID build has no reason to declare its key, so the entitlements file above leaves it out. Declaring it would take file access away from a program whose whole job is to open the files a specification names.

**No signature from a real identity, yet.** `scripts/package.sh` signs the bundle
ad-hoc unless `BRIGHTFIELD_SIGN_IDENTITY` names a certificate, and the header of
that script documents the chain that runs when one does. In a release,
`scripts/release-signing.sh` sets it: once the five repository secrets its
header names are entered, the release job imports the Developer ID certificate
into a keychain of its own and hands package.sh the identity and a notarisation
profile; with none entered it builds ad-hoc as before. The FineType extension
nested in the bundle ships as a library signed with the same identity and its
metadata trailer beside it as data, since codesign cannot sign the joined file. `signing-rehearsal.yml` runs the signed path,
with a throwaway self-signed identity, on pull requests that change signing or
the code nested in the bundle, and by hand.
