# onenote2rnote

Konvertiert handgeschriebene OneNote-Notizen (`.one`, `.onetoc2`, `.onepkg` oder ein Ordner mit `.one`-Dateien) in das Rnote-Format (`.rnote`) und erhält dabei die **Vektor-Strokes** — inklusive Farbe, Strichbreite, Transparenz und Highlighter.

Rnote ist eine quelloffene Zeichnungs-/Notizen-App, die ein offenes, versionsbasiertes Dokumentformat verwendet. Dieses Tool wandelt OneNotes proprietäre Tinten-Strukturen in Rnotes `brushstroke`-Komponenten um.

## Features

- Unterstützt `.one`-Sections, `.onetoc2`- und `.onepkg`-Notebooks sowie Ordner mit `.one`-Dateien
- Konvertiert komplette Notebook-Ordner (alle Sections) in **eine** `.rnote`-Datei **oder** pro Seite eine eigene `.rnote`-Datei
- Erhält Vektor-Strokes (kein Raster), inkl. Farbe, Breite, Transparenz und Highlighter-Erkennung
- **Medien-Unterstützung**: Bilder und eingebettete PDFs werden auf der Seite platziert (Rnote `bitmapimage`). Im Ordner wird pro Seite **eine gemergte Original-PDF** (`<Seite>-original.pdf`) abgelegt — OneNote speichert ein PDF pro Seite einzeln, das Tool setzt sie mit `pdfunite` wieder zu einem Dokument zusammen. Bilder werden als `*-media-N.png/jpg` beigelegt.
- **Standard-Layout**: unendliche Leinwand, kariertes Raster mit 22-px-Quadraten, unsichtbare Seitenränder
- **Inkrementelle Updates** (nur im `--out-dir`-Modus): nur neue/geänderte Seiten werden neu exportiert
- Automatische Seiten-Normalisierung und -Neuanordnung **oder** exakte Originalposition (`--original-pos`)
- Wählbares Seitenformat, Hintergrundmuster und DPI

## Voraussetzungen

- [Rust](https://rustup.rs/) (Cargo + Rustc), Edition 2021
- [poppler-utils](https://poppler.freedesktop.org/) für **PDF**-Medien (nur nötig, wenn deine Seiten eingebettete PDFs enthalten):
  ```sh
  sudo apt install poppler-utils
  ```

## Kompilieren

```sh
cargo build --release
```

Die fertige Binary liegt unter `target/release/onenote2rnote`.

## Verwendung

Einzelne `.one`-Section konvertieren und Seiten auflisten:

```sh
./target/release/onenote2rnote "/home/user/Documents/Physik.onepkg/Formelsammlungen.one" \
    -o ~/Documents/Formelsammlungen.rnote --list-pages
```

`--list-pages` zeigt alle Seiten samt Anzahl der Strokes/Medien und **schreibt keine Dateien** (Dry-Run).

### Standard: pro Seite eine Datei

Standardmäßig erzeugt das Tool **pro OneNote-Seite eine eigene `.rnote`** — nummeriert und nach Seitentitel benannt, in einen Ordner neben der Eingabe (Ordner = Name der `.one`-Datei):

```sh
./target/release/onenote2rnote "/home/user/Documents/Formelsammlungen.one"
```
→ erzeugt z. B. `~/Documents/Formelsammlungen/01 Formelsammlungen.rnote`, `02 ...rnote`, …

Zusätzlich werden die **Original-Medien** (Bilder/PDFs) als `*-media-N.ext` beigelegt, und unveränderte Seiten bleiben bei erneutem Lauf unberührt (inkrementell über ein Manifest).

Anderes Ausgabeverzeichnis wählen:

```sh
./target/release/onenote2rnote "/home/user/Documents/Formelsammlungen.one" --out-dir ~/Notizen/Physik
```

### Alle Seiten in eine Datei (optional)

```sh
./target/release/onenote2rnote "/home/user/Documents/Formelsammlungen.one" -o ~/Documents/Formelsammlungen.rnote
```

## CLI-Flags

| Flag | Beschreibung | Standard |
|------|--------------|----------|
| `<input>` | `.one`-Datei, `.onetoc2`/`.onepkg`-Notebook oder Ordner | – (Pflicht) |
| `--out-dir <VERZ>` | Ordner für pro-Seite-`.rnote`-Dateien + Original-Medien; inkrementell | Ordner neben Eingabe |
| `-o, --output` | Alle Seiten in **eine** `.rnote`-Datei (deaktiviert pro-Seite) | – |
| `--prune` | Dateien/Manifest-Einträge gelöschter Seiten entfernen | aus |
| `--original-pos` | Inhalte exakt am Original-Ort (keine Ränder/Neuausrichtung) | aus |
| `--format` | Seitenformat: `a4`, `us_letter`, `source` | `source` |
| `--background` | Hintergrund: `none`, `lines`, `grid` (22-px-Raster) | `grid` |
| `--margin` | Seitenrand in px um die Handschrift | `48` |
| `--dpi` | DPI des Rnote-Dokuments | `96` |
| `--min-page-height-mm` | Mindest-Seitenhöhe in mm | – |
| `--no-normalize` | Handschrift **nicht** auf das Seitenraster verschieben/ausrichten | aus |
| `--list-pages` | Zusammenfassung ausgeben, dann beenden (kein Schreiben) | – |
| `--rnote-version` | Rnote-Dateiformat-Version (muss zur installierten Rnote passen) | `0.15.0` |
| `-v, --verbose` | Detaillierte Ausgabe | – |
| `-h, --help` | Hilfe anzeigen | – |

**Wichtig (`--rnote-version`):** Die Version **muss** exakt zur installierten Rnote passen, sonst kann Rnote die Bild-Positionen nicht lesen und stapelt alle Bilder übereinander. Rnote 0.14 und 0.15 verwenden unterschiedliche Formate (0.14 verschachtelt die affine-Matrix unter `transform`, 0.15 nutzt ein flaches `affine`). Bei Rnote 0.14.x z.B.:

```sh
onenote2rnote --out-dir out --rnote-version 0.14.2 "Formelsammlungen.one"
```

**Hinweis:** Standard ist eine **unendliche Leinwand** mit kariertem 22-px-Raster und unsichtbaren Seitenrändern. Die Seitengröße (`--format`) wird nur im Ein-Datei-Modus relevant; der Inhalt wird nie aufgebläht oder gestreckt. Ein mehrseitiger PDF-Druck wird wie Rnotes eigener PDF-Import platziert: jede Seite unter der vorherigen (16-px-Abstand), das Original-PDF nur als `-original.pdf`-Sidecar.

## Ergebnis in Rnote öffnen & validieren

```sh
xdg-open ~/Documents/Formelsammlungen.rnote
```

Optional die Datei-Struktur mit der echten Rnote-Engine prüfen:

```sh
flatpak run --command=rnote-cli com.github.flxzt.rnote test ~/Documents/Formelsammlungen.rnote
```

→ „Test succeeded" bedeutet, die Datei ist gültig.

## Testen

```sh
cargo test
```

**Hinweis zu den Test-Samples:** Die Integrationstests (`tests/integration.rs`) erwarten die Dateien `tests/samples/desktop_missing_ink.one` und `tests/samples/deleted_pages.one`. Aktuell sind diese **nicht vorhanden** (das Verzeichnis `tests/samples/` enthält nur ein leeres `nb/`-Unterverzeichnis), wodurch die Tests sauber übersprungen werden. Um die Tests real auszuführen:

1. Erstelle mit OneNote eine `.one`-Datei mit Handschrift und lege sie als `tests/samples/desktop_missing_ink.one` ab.
2. Lege eine `.one`-Datei **ohne** Handschrift (z. B. mit gelöschten Seiten) als `tests/samples/deleted_pages.one` ab.

Dann werden die beiden Tests aktiv und prüfen die vollständige Rnote-Ausgabestruktur.

## Projektstruktur

```
src/
  main.rs        CLI-Einstiegspunkt (clap) & Kommandoausführung
  lib.rs         Modul-Exporte
  onedata.rs     OneNote-Eingabe parsen (.one / .onetoc2 / .onepkg / Ordner), Bilder & eingebettete Dateien
  rnote.rs       Strokes/Medien aufbereiten & Rnote-`.rnote`-Datei bauen (gzip + JSON)
  pdf.rs         eingebettete PDFs per pdftoppm zu Bitmaps rendern
  manifest.rs    Inkrementelle Synchronisation (Fingerprint/Manifest)
tests/
  integration.rs Integrationstests (Struktur & Fehlerfälle)
```

## Lizenz

GPL-3.0-or-later (siehe `Cargo.toml`).
