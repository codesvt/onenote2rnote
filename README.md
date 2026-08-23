# onenote2rnote

Konvertiert handgeschriebene OneNote-Notizen (`.one`, `.onetoc2`, `.onepkg` oder ein Ordner mit `.one`-Dateien) in das Rnote-Format (`.rnote`) und erhält dabei die **Vektor-Strokes** — inklusive Farbe, Strichbreite, Transparenz und Highlighter.

Rnote ist eine quelloffene Zeichnungs-/Notizen-App, die ein offenes, versionsbasiertes Dokumentformat verwendet. Dieses Tool wandelt OneNotes proprietäre Tinten-Strukturen in Rnotes `brushstroke`-Komponenten um.

## Features

- Unterstützt `.one`-Sections, `.onetoc2`- und `.onepkg`-Notebooks sowie Ordner mit `.one`-Dateien
- Konvertiert komplette Notebook-Ordner (alle Sections) in **eine** `.rnote`-Datei
- Erhält Vektor-Strokes (kein Raster), inkl. Farbe, Breite, Transparenz und Highlighter-Erkennung
- Automatische Seiten-Normalisierung und -Neuanordnung
- Wählbares Seitenformat, Hintergrundmuster und DPI
- Bounded Memory: gzip-Streaming der JSON-Ausgabe stroke-für-stroke

## Voraussetzungen

- [Rust](https://rustup.rs/) (Cargo + Rustc), Edition 2021

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

`--list-pages` zeigt alle Seiten samt Anzahl der Strokes pro Seite (sollten > 0 sein).

Ganzen Notebook-Ordner konvertieren (alle Sections in eine Datei):

```sh
./target/release/onenote2rnote "/home/user/Documents/Physik.onepkg" \
    -o ~/Documents/Physik.rnote
```

Ohne `-o` wird der Ausgabepfad aus dem Eingabepfad abgeleitet (Eingabe + `.rnote`).

## CLI-Flags

| Flag | Beschreibung | Standard |
|------|--------------|----------|
| `<input>` | `.one`-Datei, `.onetoc2`/`.onepkg`-Notebook oder Ordner | – (Pflicht) |
| `-o, --output` | Ausgabe-`.rnote`-Datei | Eingabepfad + `.rnote` |
| `--format` | Seitenformat: `a4`, `us_letter`, `source` | `source` |
| `--background` | Hintergrund: `none`, `lines`, `grid` | `lines` |
| `--margin` | Seitenrand in px um die Handschrift | `48` |
| `--dpi` | DPI des Rnote-Dokuments | `96` |
| `--min-page-height-mm` | Mindest-Seitenhöhe in mm | – |
| `--no-normalize` | Handschrift **nicht** auf das Seitenraster verschieben/ausrichten | aus |
| `--list-pages` | Zusammenfassung gefundener Seiten & Stroke-Zahlen ausgeben | – |
| `--rnote-version` | Rnote-Dateiformat-Version (muss zur installierten Rnote passen) | `0.15.0` |
| `-v, --verbose` | Detaillierte Ausgabe | – |
| `-h, --help` | Hilfe anzeigen | – |

**Tipp:** `--format a4` erzeugt feste, gleichmäßige Seiten. Bei `source` können sehr große OneNote-Seiten die Seiten im Rnote-Dokument stark strecken.

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
  onedata.rs     OneNote-Eingabe parsen (.one / .onetoc2 / .onepkg / Ordner)
  rnote.rs       Strokes aufbereiten & Rnote-`.rnote`-Datei bauen (gzip + JSON)
tests/
  integration.rs Integrationstests (Struktur & Fehlerfälle)
```

## Lizenz

GPL-3.0-or-later (siehe `Cargo.toml`).
