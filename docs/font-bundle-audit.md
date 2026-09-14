# Font Bundle Audit

## Reference Font Set (Linux)

| Font | Source | License | Redistributable |
|------|--------|---------|-----------------|
| Carlito | Google Fonts (based on Calibri) | Apache 2.0 | Yes |
| Caladea | Google Fonts (based on Cambria) | Apache 2.0 | Yes |
| Liberation Sans | Red Hat | LGPLv2.1+ | Yes |
| Liberation Serif | Red Hat | LGPLv2.1+ | Yes |
| Liberation Mono | Red Hat | LGPLv2.1+ | Yes |
| Noto Sans | Google | Apache 2.0 + OFL | Yes |
| Noto Serif | Google | Apache 2.0 + OFL | Yes |
| Noto Sans Mono | Google | Apache 2.0 + OFL | Yes |

## First-Run Strategy

1. On first run, `stichos doctor` checks for reference fonts via `fc-match`
2. If fonts missing, outputs diagnostic with install instructions:
   - Ubuntu/Debian: `apt install fonts-noto-core fonts-liberation`
   - Fedora: `dnf install fontconfig-devel`
   - Generic: download from google/fonts or fonts.google.com/liberation
3. Render continues without fonts (with degraded quality warnings)
4. Fonts are NOT bundled in the binary — they are soft dependencies (D8)
