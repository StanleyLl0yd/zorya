# Zorya visual identity

Zorya uses one visual identity across the repository, documentation and application shell.

## Canonical mark

The canonical repository asset is the original user-supplied PNG:

`assets/branding/zorya-icon.png`

Canonical-source properties:

- format: PNG;
- dimensions: 1254 × 1254;
- SHA-256: `88542ff96677888710940ba37fdf5fa8c99ca5ffb2a0c073c355838f0d6404fd`;
- the repository copy is stored byte-for-byte unchanged from the supplied source.

Do not re-encode, resize, crop, trace, redraw, vectorize, recolor or otherwise transform the canonical PNG unless the project owner explicitly asks for that change. Do not replace it with an SVG or with a visual reconstruction.

The Windows shell embeds this PNG and decodes those original bytes at runtime for the native window icon. The application therefore uses the canonical artwork itself rather than a separately redrawn icon.

If a platform later strictly requires a different icon container or raster size, that file is a technical derivative only. The original PNG remains the canonical source, must remain present unchanged, and must not be silently replaced by the derivative.

The mark is a sunrise over a dark horizon inside a rounded midnight-blue field, framed by a violet orbit and warm solar light.

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

Future installers, shortcuts, store imagery, screenshots and release graphics should use the canonical artwork and the visual language around it. Do not substitute temporary generic browser icons or independently redrawn variants in public release assets.

Do not modify the canonical PNG merely to satisfy a platform export requirement. When a technically required derivative is unavoidable, generate it from the canonical PNG, identify it as a derivative, and keep the canonical PNG unchanged.

## Tone

The visual language should match the project itself: precise, calm, technical and restrained. It should communicate an early but deliberately engineered browser rather than imply maturity that the current Technical Preview does not have.
