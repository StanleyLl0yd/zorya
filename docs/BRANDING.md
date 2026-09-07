# Zorya visual identity

Zorya uses one visual identity across the repository, documentation and application shell.

## Canonical mark

The canonical repository asset is:

`assets/branding/zorya-icon.svg`

The mark is a sunrise over a dark horizon inside a rounded midnight-blue field, framed by a violet orbit and warm solar light. Platform-specific renditions may change raster size or padding, but must preserve that composition.

The Windows shell derives its native window icon from the same geometry in `src/branding.rs`; this keeps the app icon dependency-free and reproducible from source.

## Palette

| Role | Color |
| --- | --- |
| Midnight base | `#07103B` |
| Deep indigo | `#0C1550` |
| Electric violet | `#6F4CFF` |
| Soft violet | `#A58CFF` |
| Sunrise orange | `#FF9B35` |
| Warm amber | `#FFBD4A` |
| Solar highlight | `#FFF3B0` |
| Light text | `#F7F2FF` |

The dark blues are structural. Violet is used for borders, orbit lines and focus accents. Orange/amber is reserved for important highlights and the sunrise motif.

## Shape language

Use:

- rounded rectangles and restrained large-radius corners;
- arcs, horizon curves and orbital lines;
- thin luminous outlines rather than heavy borders;
- a dark field with small, high-contrast warm accents;
- generous empty space;
- simple geometric compositions.

Avoid:

- unrelated stock illustrations;
- bright flat-white layouts as the primary visual identity;
- random accent colors outside the palette;
- dense neon decoration that competes with content;
- recreating the mark differently in each surface.

## Repository and documentation

The main README must display the canonical mark near the title.

Visual documentation, screenshots, social previews and future banners should use the same midnight/violet/sunrise system. Markdown documents remain content-first: branding must not reduce readability or turn engineering documentation into decorative artwork.

## Application shell

The native application uses the Zorya mark as its window icon and the startup surface uses the same core palette.

Browser chrome added during Z2 and later should follow the same hierarchy:

- privileged shell background: midnight/deep indigo;
- active/focus state: violet;
- primary highlight or progress/loading emphasis: sunrise amber;
- readable text: near-white;
- Web content remains visually and architecturally separate from privileged chrome.

The branding palette is a product presentation rule, not a Web-engine rendering rule. Rarog must not contain Zorya-specific branding.

## Release assets

Future installers, shortcuts, store imagery, screenshots and release graphics should be derived from the canonical mark. Do not substitute temporary generic browser icons in public release assets.

If an OS requires a different format such as ICO or PNG, generate it from the canonical mark and preserve the source asset in the repository.

## Tone

The visual language should match the project itself: precise, calm, technical and restrained. It should communicate an early but deliberately engineered browser rather than imply maturity that the current Technical Preview does not have.
