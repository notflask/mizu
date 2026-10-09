# mizu: Implementierungsplan

> **mizu** (水, Wasser) ist ein sehr minimalistischer PDF-Viewer fürs Studium.
> Er kann Vim-Keybindings, einen Dunkelmodus für das PDF selbst, Zoom ohne Ruckeln
> und Malen mit Stift und Radierer inklusive Speichern.
> Plattformen: **Linux (Hauptziel: NixOS, Wayland, Niri/Hyprland)**, Windows, macOS.

Dieses Dokument ist die verbindliche Spezifikation für die Umsetzung. Es richtet sich an
einen Coding-Agenten bzw. Entwickler, der das Projekt Meilenstein für Meilenstein baut.

---

## 0. Hinweise für den umsetzenden Agenten

- **Arbeite strikt Meilenstein für Meilenstein** (Abschnitt 17). Am Ende jedes Meilensteins:
  `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. Erst wenn alles grün ist,
  committen. Ein Commit pro abgeschlossenem Teilschritt.
- **Crate-APIs vor Benutzung auf docs.rs prüfen.** Versionen ändern sich, und Code „aus dem
  Gedächtnis“ ist oft veraltet. Das gilt besonders für `winit` (ab 0.30 mit `ApplicationHandler`-API),
  `wgpu` und `mupdf`. Nimm jeweils die aktuelle stabile Version und pinne sie über `Cargo.lock`
  (wird committet).
- **`glyphon` muss zur `wgpu`-Version passen.** Wähle die `wgpu`-Version, die die aktuelle
  `glyphon`-Version verlangt.
- Fehlerbehandlung mit `anyhow` (App) bzw. `thiserror` (Module mit eigenen Fehlertypen).
  Bei Benutzereingaben, Dateien und PDFs **niemals `unwrap()`**: Fehler kommen als Meldung in die Statuszeile.
- Logging über `log` + `env_logger` (`RUST_LOG=mizu=debug`).
- Keine zusätzlichen Features erfinden. Was nicht in diesem Plan steht, wird nicht gebaut
  (siehe Nicht-Ziele, Abschnitt 2).
- Code und Kommentare auf Englisch, Benutzertexte (Statuszeile, Fehlermeldungen) auf Englisch.

---

## 1. Entscheidungen (mit dem Nutzer abgestimmt)

| Thema | Entscheidung |
|---|---|
| Sprache | **Rust** (stable, Edition 2021 oder neuer) |
| PDF-Engine | **MuPDF** über das Crate `mupdf` (mupdf-rs, baut MuPDF statisch aus dem Quellcode) |
| Lizenz | **AGPL-3.0-or-later** (wegen MuPDF); `LICENSE`-Datei anlegen |
| Fenster/Input | `winit` |
| Grafik | `wgpu` (Vulkan/Metal/DX12, GL als Fallback) |
| UI-Stil | Wie **sioyek**: keine Toolbar und keine Menüs, nur Seiten, eine einzeilige Statuszeile und eine `:`-Kommandozeile |
| Dunkelmodus | Färbt **alles** im PDF um, auch Bilder. Der Farbton bleibt erhalten, die Helligkeit wird invertiert |
| Zeichnen | Nur **Stift** (Farbe, Dicke) und **Radierer**, mit **Undo/Redo**. Kein Textmarker, keine Textnotizen, keine Formen |
| Eingabegerät | Maus. Ein Stift funktioniert als Maus; Druckstufen sind optional (M8) |
| Speichern | **Wie Vim**: `:w` überschreibt die Originaldatei, kein Autosave. Striche werden als echte PDF-Ink-Annotationen gespeichert |
| Ansicht | Ein Dokument pro Fenster, durchgehend vertikales Scrollen, **keine** Doppelseitenansicht |
| Icon | Seite mit Eselsohr, durch die eine Welle läuft: oben hell, unten dunkel („eingetaucht“) |

---

## 2. Nicht-Ziele

Diese Features werden **nicht** gebaut: Textauswahl und Kopieren, Text-Highlight-Annotationen,
Textnotizen, Formen, Doppelseitenansicht, Tabs, Präsentationsmodus, Formulare ausfüllen,
Drucken, eingebauter Dateidialog, Thumbnails-Seitenleiste, Plugin-System, EPUB und andere Formate.

---

## 3. Projektstruktur

```
mizu/
├── Cargo.toml
├── Cargo.lock
├── LICENSE                       # AGPL-3.0-or-later
├── README.md                     # Kurzbeschreibung, Installation, Keybindings
├── flake.nix / flake.lock        # NixOS: devShell + Paket
├── build.rs                      # Windows: Icon in .exe einbetten
├── docs/PLAN.md
├── assets/
│   ├── fonts/                    # eine kleine OFL-Schrift für die Statuszeile (z. B. Inter oder JetBrains Mono)
│   └── icons/
│       ├── src/                  # SVG-Quellen (Hand gepflegt)
│       └── generated/            # erzeugte PNG/ICO/ICNS (committet)
├── packaging/
│   ├── linux/io.github.notflask.Mizu.desktop
│   ├── macos/Info.plist
│   └── windows/mizu.rc (falls nötig)
├── xtask/                        # Hilfsprogramm: Icons generieren (`cargo xtask icons`)
├── tests/fixtures/               # kleine Test-PDFs (inkl. rotierte Seite, gemischte Seitengrößen)
└── src/
    ├── main.rs                   # CLI-Parsing, Logger, Event-Loop starten
    ├── app.rs                    # winit ApplicationHandler, verbindet alle Teile
    ├── config.rs                 # config.toml laden, Defaults
    ├── session.rs                # letzte Position pro Datei (XDG state dir)
    ├── doc/
    │   ├── mod.rs                # Document-Metadaten (Seitenanzahl, Seitengrößen), Öffnen
    │   ├── worker.rs             # Render-Worker-Pool, Job-Queue, Abbruch veralteter Jobs
    │   ├── annots.rs             # mizu-Striche aus PDF laden / in PDF speichern
    │   ├── ffi_ext.rs            # fehlende MuPDF-Funktionen über mupdf-sys (unsafe, gekapselt)
    │   ├── search.rs
    │   ├── outline.rs
    │   └── links.rs
    ├── view/
    │   ├── layout.rs             # Seiten untereinander anordnen (Dokumentkoordinaten)
    │   └── camera.rs             # Offset + Zoom, Umrechnung Bildschirm <-> Dokument <-> Seite
    ├── render/
    │   ├── gpu.rs                # wgpu Device/Surface/MSAA-Target
    │   ├── tiles.rs              # Tile-Textur-Cache (LRU, Speicherbudget)
    │   ├── pages.rs + pages.wgsl # Seiten-Quads zeichnen, Recolor im Shader
    │   ├── strokes.rs + strokes.wgsl # Strich-Geometrie (lyon), Recolor im Shader
    │   ├── recolor.wgsl          # gemeinsame Recolor-Funktion (per include/concat)
    │   └── ui.rs                 # Statuszeile, Kommandozeile, Overlays (glyphon)
    ├── input/
    │   ├── keys.rs               # Parser für "<C-d>", "gg", "<Space>" usw.
    │   ├── keymap.rs             # Modus-abhängige Keymaps, Präfix-Sequenzen, Counts
    │   ├── actions.rs            # enum Action
    │   └── command.rs            # ":"-Kommandos parsen, Tab-Completion für Pfade
    ├── ink/
    │   ├── stroke.rs             # Stroke-Datenmodell
    │   ├── smooth.rs             # Glättung + Vereinfachung (RDP)
    │   ├── eraser.rs             # Hit-Test
    │   └── history.rs            # Undo/Redo
    └── watch.rs                  # Datei-Überwachung für Auto-Reload
```

App-ID (Wayland `app_id`, Desktop-Datei, macOS Bundle-ID): **`io.github.notflask.Mizu`**.

---

## 4. Abhängigkeiten (Richtwerte, Versionen beim Start prüfen)

| Crate | Zweck |
|---|---|
| `mupdf` (+ `mupdf-sys` für Lücken) | PDF öffnen, rendern, Annotationen, Suche, Outline, Links |
| `winit` | Fenster, Tastatur, Maus, Touch, DnD |
| `wgpu` | Rendering |
| `glyphon` | Text für Statuszeile und Overlays |
| `lyon_tessellation` | Strich-Tessellierung |
| `bytemuck` | Vertex-Daten |
| `crossbeam-channel` | Job- und Ergebnis-Queues |
| `notify` + `notify-debouncer-mini` | Auto-Reload |
| `serde`, `toml` | Config |
| `serde_json` | Session-Datei |
| `directories` | Config- und State-Pfade plattformgerecht |
| `lexopt` oder `clap` (derive) | CLI |
| `anyhow`, `thiserror`, `log`, `env_logger` | Infrastruktur |
| `open` | externe Links öffnen |
| `uuid` | IDs für Striche (`/NM`) |
| `tempfile` | atomisches Speichern |
| `pollster` | wgpu-Init blockierend |
| Dev/xtask: `resvg`, `tiny-skia`, `usvg`, `ico`, `icns` | Icon-Generierung |
| Build (Windows): `embed-resource` oder `winresource` | Icon in .exe |

---

## 5. Koordinatensysteme und Layout

Drei Räume, sauber getrennt in `view/camera.rs`:

1. **Seitenraum** (pro Seite): Einheit = PDF-Punkt (1/72 Zoll), Ursprung oben links, y nach unten.
   Das ist MuPDFs „fitz space“ nach Anwendung der Seitenrotation, also das, was
   `page.bounds()` liefert. **Alle Striche werden im Seitenraum gespeichert.**
2. **Dokumentraum**: Alle Seiten liegen untereinander, horizontal zentriert auf die breiteste Seite,
   mit `page_gap = 8 pt` Abstand. `layout.rs` hält pro Seite `origin_y` und `size`.
3. **Bildschirmraum**: physische Pixel.
   `screen = (doc - camera.offset) * camera.zoom * scale_factor`.

Kamera:
- `zoom` = logische Pixel pro Punkt. Erlaubt sind 0.1 bis 32.0.
- **Zoom zum Cursor**: Der Dokumentpunkt unter dem Cursor bleibt beim Zoomen fix (Unit-Test!).
- Zoommodi: `FitWidth` (Standard), `FitPage`, `Free`. In `FitWidth` und `FitPage` wird der Zoom
  bei Fenstergrößenänderung neu berechnet. Das passiert unter Niri/Hyprland oft, deshalb wird das
  Nachrendern entprellt (siehe 6).
- „Aktuelle Seite“ = die Seite, die die vertikale Bildschirmmitte schneidet.
- Fensterränder clampen: Man kann nicht endlos über Anfang und Ende hinausscrollen. Horizontal ist
  nur Scrollen möglich, wenn der Inhalt breiter als das Fenster ist; sonst wird zentriert.

---

## 6. Rendering und Zoom (Kernqualität!)

**Ziel:** Text ist bei jedem Zoom gestochen scharf, und Scrollen und Zoomen laufen immer in
Bildwiederholrate. Wenn noch keine scharfen Kacheln da sind, wird kurz etwas unscharf angezeigt,
aber das Bild bleibt nie leer und ruckelt nie.

### 6.1 Worker-Pool
- MuPDF-Objekte sind nicht `Send`. Deshalb öffnet **jeder Worker-Thread das Dokument selbst**.
  Es gibt `N = clamp(cores - 1, 1, 4)` Worker, und der Hauptthread hält nur Metadaten.
- Jobs: `RenderTile { page, scale, tile_x, tile_y, generation }`, `RenderThumb { page }`,
  `Search { ... }`. Ergebnisse gehen über einen Channel zurück. Danach weckt
  `EventLoopProxy::send_event` die Event-Loop.
- **Abbruch**: Bei jeder Kamera-Änderung zählt `generation` hoch. Worker verwerfen Jobs mit
  veralteter Generation, bevor sie rendern. Die Queue ist eine Prioritätsqueue: sichtbare
  Kacheln zuerst (von der Bildschirmmitte nach außen), dann Prefetch (je ein Bildschirm darüber
  und darunter).
- Pro Worker wird eine Display-List pro Seite gecacht (`Page::to_display_list`, LRU über
  ~8 Seiten). Kacheln rendern dann aus der Display-List mit Clip auf das Kachel-Rechteck.

### 6.2 Kacheln
- Kachelgröße: 512×512 physische Pixel.
- `render_scale = zoom * scale_factor`. Gerendert wird mit **exakt** diesem Wert, damit der Text
  nicht durch Resampling weich wird. Neue Jobs erst, wenn sich der Zoom **120 ms** nicht mehr
  geändert hat.
- **Während des Zoomens** werden die vorhandenen Kacheln der nächstliegenden Skala gestreckt bzw.
  gestaucht angezeigt (lineares Filtern).
- **Fallback-Ebene**: Pro Seite wird immer eine kleine Vorschau-Textur gehalten (Breite ca.
  400 px, für alle Seiten im Hintergrund vorgerendert, nachdem die sichtbaren Kacheln fertig
  sind). So ist beim schnellen Scrollen (`G`, `gg`) sofort etwas zu sehen.
- Cache: Schlüssel `(page, scale_bits, tx, ty)`, LRU mit GPU-Speicherbudget **384 MB**
  (konfigurierbar). Vorschaubilder fallen nicht unter die LRU.
- Pixmap: MuPDF rendert RGB ohne Alpha auf weißem Grund. Auf dem Worker wird das zu RGBA8
  expandiert, dann als `Rgba8UnormSrgb` hochgeladen. Der Shader sieht also lineare Werte.
- Anti-Aliasing: MuPDF-Default (8 Bit). Kein eigenes Gamma-Gefummel.

### 6.3 Frame-Ablauf
- `ControlFlow::Wait`. Neu gezeichnet wird nur bei Input, Animation, eintreffender Kachel oder Resize.
- Reihenfolge pro Frame:
  1. Hintergrund, im Dunkelmodus in der Hintergrundfarbe
  2. für jede sichtbare Seite: zuerst die Vorschau, darüber die besten verfügbaren Kacheln
  3. die Striche der Seite
  4. eventuelle Suchtreffer
  5. die Statuszeile
- MSAA 4× für die Striche. Die Seitentexturen werden ohne MSAA in dasselbe Target gezeichnet;
  dafür reicht ein MSAA-Target mit Resolve.
- Present-Mode: `AutoVsync`.

---

## 7. Dunkelmodus (Recolor)

- Er gilt **nur für den Seiteninhalt** (inklusive Bildern und Strichen), nicht für die Statuszeile.
  Die Statuszeile hat ein eigenes, festes, dezentes Farbschema, das sich an den Modus anpasst.
- Er ist eine reine Darstellungssache. Gespeicherte Strichfarben sind immer die Originalfarben.
- Die Funktion `recolor(rgb_linear) -> rgb_linear` wird im Seiten- **und** im Strich-Shader benutzt:

```
// in OKLab
lab      = oklab_from_linear_srgb(c)
fg, bg   = oklab der konfigurierten Farben (Uniforms; bg = Seitenhintergrund im Dunkelmodus)
t        = lab.L                       // 1 = weiß, 0 = schwarz
L'       = mix(fg.L, bg.L, t)          // weiß -> bg, schwarz -> fg
tint     = mix(fg.ab, bg.ab, t)        // neutrale Töne -> exakt Verlauf fg..bg
ab'      = lab.ab + tint
out      = linear_srgb_from_oklab(L', ab'), auf [0,1] clampen
```

- Eigenschaften (als CPU-Referenzimplementierung in Rust **unit-getestet**):
  Weiß wird exakt zu `bg`, Schwarz exakt zu `fg`, und gesättigtes Rot bleibt rötlich
  (Hue-Differenz < 15°).
- Standardfarben: `bg = #1b1b1d`, `fg = #dcdcdc` (konfigurierbar).
- Umschalten: Taste `D` bzw. `:dark`, `:light`. Der Zustand wird pro Datei in der Session
  gespeichert, global gilt der Default aus der Config.
- Umschalten ändert nur ein Uniform. Es wird **nichts** neu gerendert.

---

## 8. Eingabe und Vim-Keybindings

### 8.1 Modi
`Normal`, `Draw`, `Command` (nach `:`), `Search` (nach `/` oder `?`), `Outline` (Overlay).

### 8.2 Keymap-Engine (`input/keymap.rs`)
- Key-Notation wie in Vim: `j`, `G`, `<C-d>`, `<S-Space>`, `<Esc>`, `<Tab>`, `<CR>`, `<F5>`.
  Parser und Formatter werden unit-getestet.
- **Count-Präfix**: `5j`, `42G`, `3<C-d>`. Eine Ziffer `0` ohne vorherige Ziffer ist eine normale Taste.
- **Präfix-Sequenzen**: `gg`, `zw`, `ZZ`, `m{a-z}`, `'{a-z}`. Es gibt keinen Timeout. Ist eine
  Sequenz eindeutig ein Präfix, wird gewartet. Ist sie ungültig, wird sie verworfen.
- Die Keymap ist eine Tabelle `mode -> (sequence -> Action)`, kommt aus Defaults und wird von
  `config.toml` überschrieben bzw. erweitert. Im `Draw`-Modus gelten zusätzlich alle
  Navigationstasten aus `Normal`; Draw-spezifische Tasten haben Vorrang.
- Tasten-Wiederholung (gedrückt halten) funktioniert für Bewegungen.

### 8.3 Default-Belegung: Normal

| Taste | Aktion |
|---|---|
| `j` / `k` | 1 Schritt runter / hoch (Schritt = 60 logische px, × Count) |
| `h` / `l` | 1 Schritt links / rechts |
| `<C-d>` / `<C-u>` | halbe Fensterhöhe runter / hoch |
| `<C-f>` / `<C-b>`, `<Space>` / `<S-Space>` | ganze Fensterhöhe (minus 1 Zeile Überlappung) |
| `J` / `K` | nächste / vorherige Seite (Seitenanfang an den oberen Rand) |
| `gg` / `G` | Anfang / Ende. Mit Count: zu Seite N (`42G`, `42gg`) |
| `+` / `-` | Zoom × 1.2 / ÷ 1.2 (um die Fenstermitte) |
| `=` / `zw` | Seitenbreite |
| `zf` | ganze Seite |
| `z0` | 100 % |
| `/` / `?` | Suche vorwärts / rückwärts |
| `n` / `N` | nächster / vorheriger Treffer |
| `m{a-z}` | Marke setzen (Seite + Offset), wird in der Session gespeichert |
| `'{a-z}` und `` `{a-z} `` | zur Marke springen |
| `<C-o>` / `<C-i>` (`<Tab>`) | Sprungliste zurück / vor. Gefüllt durch `gg`, `G`, `NG`, Marken, Suche, Links, Outline |
| `o` | Inhaltsverzeichnis (Outline-Overlay) |
| `D` | Dunkelmodus an/aus |
| `i` | Zeichenmodus (Stift) |
| `u` / `<C-r>` | Undo / Redo |
| `r` | Datei neu laden |
| `:` | Kommandozeile |
| `ZZ` | wie `:x` |
| `ZQ` | wie `:q!` |
| `<Esc>` | Suchtreffer-Hervorhebung und Meldung ausblenden |

### 8.4 Default-Belegung: Draw

| Taste / Maus | Aktion |
|---|---|
| `<Esc>` | zurück zu Normal |
| Linke Maustaste ziehen | zeichnen (oder radieren, wenn der Radierer aktiv ist) |
| Rechte Maustaste ziehen | temporär radieren |
| Mittlere Maustaste ziehen | verschieben |
| `e` | Stift ↔ Radierer umschalten |
| `p` | Stift |
| `1`…`9` | Farbe aus der Palette |
| `[` / `]` | Dicke ÷ 1.25 / × 1.25 (Grenzen 0.25 bis 20 pt) |
| `u` / `<C-r>` | Undo / Redo |
| alle Navigationstasten aus Normal | wie in Normal (außer `i`, `e`, `p`, Ziffern) |

Hinweis: Ziffern sind im Draw-Modus Farben und keine Counts.

### 8.5 Maus und Touchpad (alle Modi)
- Mausrad: scrollen. `LineDelta` × Schrittweite, `PixelDelta` (Touchpad) 1:1 und weich.
- `Ctrl` + Mausrad: Zoom zum Cursor (Faktor 1.1 pro Raste, kontinuierlich bei `PixelDelta`).
- Pinch-Geste: Zoom zum Pinch-Mittelpunkt, sofern `winit` sie auf der Plattform liefert
  (`PinchGesture`). Auf Wayland ist sie in winit eventuell nicht verfügbar. Das ist dann so;
  `Ctrl`+Rad und Tasten reichen.
- Normal-Modus: Ziehen mit links oder Mitte verschiebt die Ansicht. Klick ohne Ziehen auf einen
  Link folgt dem Link.
- Drag & Drop einer PDF-Datei ins Fenster öffnet sie (wie `:e`).

### 8.6 Kommandos (`:`)

| Kommando | Wirkung |
|---|---|
| `:w` | speichern (Original überschreiben, atomar) |
| `:w <pfad>` | Kopie mit Strichen nach `<pfad>` schreiben; die aktuelle Datei bleibt dieselbe (wie Vim) |
| `:q` | beenden. Bei ungespeicherten Änderungen: `E37: No write since last change (add ! to override)` |
| `:q!` | beenden ohne Speichern |
| `:wq`, `:x` | speichern und beenden (`:x` speichert nur bei Änderungen) |
| `:e <pfad>` | andere Datei öffnen (mit derselben Dirty-Prüfung wie `:q`) |
| `:e!` | neu laden und Änderungen verwerfen |
| `:<N>` | zu Seite N |
| `:dark`, `:light` | Dunkelmodus an / aus |
| `:color #rrggbb` | Stiftfarbe setzen |
| `:width <pt>` | Stiftdicke setzen |

- `<Tab>` vervollständigt Pfade bei `:e` und `:w`; `~` wird expandiert.
- `<Up>` / `<Down>` blättern durch die Kommando-Historie (pro Sitzung).
- `<Esc>` bricht ab, `<CR>` führt aus, `<BS>` auf einer leeren Zeile beendet den Modus.
- **Fenster schließen (CloseRequested)** bei ungespeicherten Änderungen: Das erste Mal erscheint
  nur die E37-Meldung. Ein zweiter Schließversuch innerhalb von 3 s beendet das Programm.

---

## 9. Zeichnen, Radieren, Undo/Redo

### 9.1 Datenmodell (`ink/stroke.rs`)
```rust
struct Stroke {
    id: Uuid,              // landet als /NM "mizu-<uuid>" im PDF
    page: usize,
    points: Vec<[f32; 2]>, // Seitenraum, Punkte
    width: f32,            // in pt (skaliert mit Zoom)
    color: [u8; 3],        // sRGB, Originalfarbe
}
```

### 9.2 Stift
- Ein Strich beginnt beim Drücken über einer Seite und gehört zu **dieser** Seite. Punkte
  außerhalb der Seite werden auf den Seitenrand geclamped.
- Neue Punkte werden nur übernommen, wenn sie ≥ 0,75 physische px vom letzten entfernt sind.
- Während des Zeichnens wird der Strich live tesselliert und gezeichnet. Die Latenz muss minimal
  sein: Bei jedem `CursorMoved` wird sofort neu gezeichnet.
- Beim Loslassen:
  1. leichte Glättung (Chaikin, 1 bis 2 Iterationen, oder gleitender Mittelwert über 3 Punkte;
     Endpunkte bleiben erhalten)
  2. Vereinfachung mit Ramer–Douglas–Peucker, ε = 0,1 pt (in Seitenkoordinaten, also unabhängig vom Zoom)
  3. Ein Klick ohne Bewegung ergibt einen Punkt, gezeichnet als kleiner Kreis
- Darstellung: `lyon` StrokeTessellator mit runden Caps und Joins und Breite = `width`.
  Vertex-Buffer werden pro Strich gecacht (Seitenraum, die Transformation macht der Shader).
- Cursor im Draw-Modus: Kreis mit dem Durchmesser der aktuellen Dicke (in Bildschirm-px) in der
  aktuellen Farbe. Im Radierer-Modus ein Kreis-Outline mit dem Radierer-Radius.
- **Palette** (Defaults, in der Config änderbar):
  1. `#1a1a1a` (schwarz)
  2. `#e03131` (rot)
  3. `#1971c2` (blau)
  4. `#2f9e44` (grün)
  5. `#f08c00` (orange)
  6. `#9c36b5` (lila)
- Default-Dicke: 1,5 pt.

### 9.3 Radierer (`ink/eraser.rs`)
- **Strich-Radierer**: Jeder Strich, dessen Polylinie (plus halbe Strichbreite) den Radierkreis
  berührt, wird komplett entfernt.
- Radius: 10 logische px, umgerechnet in pt beim aktuellen Zoom.
- Hit-Test: Abstand Punkt–Segment. Vorfilter über die Bounding-Box pro Strich.
- Alle während **eines** Ziehens entfernten Striche ergeben **ein** Undo-Schritt.
- Radiert werden nur mizu-eigene Striche, keine fremden Annotationen.

### 9.4 Undo/Redo (`ink/history.rs`)
- `enum Edit { Add(Stroke), Remove(Vec<Stroke>) }`, Undo-Stack und Redo-Stack, unbegrenzt pro Sitzung.
- Eine neue Änderung leert den Redo-Stack.
- **Dirty-Flag**: Die Undo-Stack-Länge beim letzten Speichern wird gemerkt. Dirty heißt, die
  aktuelle Länge oder der Inhalt weicht davon ab (Implementierung über einen Zähler
  `saved_revision` vs. `revision`).
- `u` und `<C-r>` funktionieren in Normal **und** Draw. Nach Undo/Redo springt die Ansicht
  **nicht** (anders als Vim). Ist die betroffene Seite nicht sichtbar, kommt eine Meldung
  `Undo on page 12`.

---

## 10. Speichern und Laden der Striche (`doc/annots.rs`)

### 10.1 Format im PDF
Jeder Strich wird eine Standard-**Ink-Annotation**. Damit erscheint er auch in Okular, Zathura,
Acrobat und Browsern.
- `/Subtype /Ink`
- `/InkList [[x1 y1 x2 y2 …]]`
- `/C [r g b]` (0..1)
- `/BS << /W width >>`
- `/CA 1`
- `/NM (mizu-<uuid>)`: **daran erkennt mizu eigene Striche**
- `/T (mizu)`
- Appearance-Stream von MuPDF generieren lassen (`pdf_update_annot`), damit andere Viewer ihn
  korrekt anzeigen.
- MuPDFs Annotations-Funktionen erwarten Koordinaten im fitz-Seitenraum und rechnen selbst in den
  PDF-Raum um (inklusive `/Rotate` und CropBox). Das wird mit der rotierten Fixture-Seite
  **getestet**.
- Falls `mupdf-rs` die nötigen Funktionen nicht anbietet (`pdf_set_annot_ink_list`,
  `pdf_set_annot_color`, `pdf_set_annot_border_width`, `/NM` setzen bzw. lesen,
  `pdf_delete_annot`, Ink-List lesen), werden sie in `doc/ffi_ext.rs` über `mupdf-sys` ergänzt.
  Dort liegt ein dünner, sicherer Wrapper; alles `unsafe` bleibt in dieser Datei.

### 10.2 Laden
1. Beim Öffnen liest der Hauptthread (über einen dedizierten Lade-Job) alle Ink-Annotationen mit
   `/NM` beginnend mit `mizu-` und erzeugt daraus `Stroke`s.
2. **Jeder Render-Worker löscht diese Annotationen in seiner eigenen In-Memory-Kopie**, bevor er
   eine Seite rendert (einmal pro Seite). mizu-Striche zeichnet also nur die GPU-Ebene; sie
   erscheinen nie doppelt und bleiben editierbar.
3. Fremde Annotationen (von anderen Programmen) rendert MuPDF ganz normal. mizu kann sie nicht bearbeiten.

### 10.3 Speichern (`:w`)
1. Läuft auf einem eigenen Thread. Die UI blockiert nicht; die Statuszeile zeigt `Saving…`.
2. Die Datei wird frisch von der Festplatte geöffnet, alle `mizu-`-Annotationen werden gelöscht
   und alle aktuellen Striche als neue Ink-Annotationen angelegt.
3. Gespeichert wird in eine temporäre Datei **im selben Verzeichnis** (`tempfile::NamedTempFile::new_in`),
   als vollständiger Save ohne Garbage-Collection-Optionen, die die Struktur stark umbauen.
   Danach `fsync` und `persist` (atomisches Rename) über das Original. Dateirechte des Originals
   werden übernommen.
4. Bei Erfolg: `saved_revision = revision` und Meldung `"skript.pdf" written`. Bei Fehler (z. B.
   verschlüsseltes PDF, keine Schreibrechte) erscheint die Meldung in der Statuszeile, und das
   Dirty-Flag bleibt.
5. Das Auto-Reload (11.3) ignoriert das Dateiereignis des eigenen Speicherns. Dafür wird ein
   Flag bzw. Zeitstempel gesetzt und die mtime nach dem Rename verglichen.

---

## 11. Weitere Viewer-Funktionen

### 11.1 Suche
- `/text<CR>` sucht ab der aktuellen Seite vorwärts (`?` rückwärts) mit `Page::search`, seitenweise
  auf einem Worker. Ergebnisse kommen inkrementell.
- Groß- und Kleinschreibung: Smartcase wie in Vim (enthält die Suche Großbuchstaben, ist sie
  case-sensitiv). Falls MuPDF nur case-insensitiv sucht, wird für den case-sensitiven Fall
  nachgefiltert.
- Treffer werden als halbtransparente Rechtecke (Quads) gezeichnet. Der aktuelle Treffer ist
  kräftiger hervorgehoben.
- `n` / `N` springen zum Treffer und zentrieren ihn vertikal. Die Statuszeile zeigt `[3/17]`.
- Kein Treffer: `E486: Pattern not found: text`.

### 11.2 Outline (Inhaltsverzeichnis), wie sioyek
- `o` öffnet ein Overlay mit einer Liste, eingerückt nach Ebene, plus Seitenzahl.
- `j` / `k` (bzw. Pfeiltasten) wählen aus, `<CR>` springt (Eintrag in die Sprungliste),
  `<Esc>` schließt. Tippen filtert die Liste (case-insensitive Substring).
- Gibt es keine Outline: Meldung `No outline`.

### 11.3 Auto-Reload
- `notify` beobachtet das **Verzeichnis** der Datei, weil Editoren und LaTeX oft per Rename
  ersetzen. Ereignisse werden auf 200 ms entprellt.
- Beim Reload werden Seite, Offset und Zoom beibehalten, alle Caches verworfen und die Striche
  neu geladen.
- Gibt es **ungespeicherte Striche**, wird nicht automatisch neu geladen. Stattdessen erscheint
  `File changed on disk. :e! to reload (discards drawings)`.

### 11.4 Links
Klick im Normal-Modus:
- interne Links springen (Sprungliste)
- externe Links öffnen über `open::that`

Fährt man über einen Link, wird der Cursor zur Hand.

### 11.5 Session (`session.rs`)
- Datei: `<state_dir>/mizu/session.json`, auf Linux `~/.local/state/mizu/`.
- Pro kanonischem Dateipfad: Seite + Offset, Zoom-Modus/Zoom, Dunkelmodus, Marken.
- Wird beim Schließen und beim Wechsel der Datei geschrieben, atomisch.
- Maximal 500 Einträge (LRU).

### 11.6 Passwortgeschützte PDFs
Braucht das Dokument ein Passwort, fragt die Kommandozeile `Password: `. Die Eingabe wird als `*`
angezeigt. Drei Versuche, danach `Wrong password` und ein leeres Fenster.

---

## 12. Statuszeile und Oberfläche

- Eine Zeile am unteren Rand. Höhe = Schrifthöhe + 6 px. Die Schrift ist eingebettet
  (`include_bytes!`) und **nicht** von Systemschriften abhängig, weil diese unter NixOS
  unzuverlässig gefunden werden.
- Links:
  - der Modus (`NORMAL` / `DRAW` / `ERASE`)
  - der Dateiname
  - `[+]`, wenn dirty
- Rechts:
  - im Draw-Modus ein Farbpunkt und die Dicke (`● 1.5pt`)
  - Suchstatus
  - `12/148`
  - `125%`
- Meldungen und Fehler ersetzen den linken Teil, bis zur nächsten Taste bzw. 4 s lang.
  Fehler werden rot dargestellt.
- Im Command- und Search-Modus wird die Zeile zur Eingabezeile (`:` bzw. `/` plus Text und Cursor).
- Mit `statusbar = false` in der Config ist die Statuszeile ausgeblendet. Sie erscheint dann nur
  bei Eingaben und Meldungen.
- Ohne Datei (Start ohne Argument): leeres Fenster, zentriert dezent `mizu — :e <file>`.
- Fenstertitel: `skript.pdf — mizu`.
- **Wayland**: Die `app_id` wird über
  `WindowAttributesExtWayland::with_name("io.github.notflask.Mizu", "mizu")` gesetzt, damit
  Niri und Hyprland Fensterregeln und Icon zuordnen können.

---

## 13. Konfiguration

Pfad: `directories::ProjectDirs` → Linux `~/.config/mizu/config.toml`, macOS
`~/Library/Application Support/io.github.notflask.Mizu/config.toml`, Windows
`%APPDATA%\notflask\Mizu\config\config.toml`.

- Fehlt die Datei, gelten die Defaults.
- Ist die Datei ungültig, gelten ebenfalls die Defaults, und die Statuszeile zeigt den Parse-Fehler.
- `README.md` dokumentiert ein vollständiges Beispiel:

```toml
dark_by_default = false
statusbar = true
scroll_step = 60          # logische px
zoom_step = 1.2
tile_cache_mb = 384

[dark]
background = "#1b1b1d"
foreground = "#dcdcdc"

[pen]
width = 1.5
palette = ["#1a1a1a", "#e03131", "#1971c2", "#2f9e44", "#f08c00", "#9c36b5"]

[keys.normal]
"<C-n>" = "toggle_dark"   # eigene Belegungen überschreiben/ergänzen Defaults
"D" = "none"              # Default entfernen

[keys.draw]
"x" = "toggle_eraser"
```

Action-Namen sind die `snake_case`-Varianten von `enum Action`. Sie stehen in `README.md` als
Tabelle (generiert oder von Hand, Hauptsache vollständig).

---

## 14. CLI

```
mizu [FILE] [--page N]
mizu --version | --help
```
`--page` überschreibt die gespeicherte Session-Position. Exit-Code 0, außer beim Fehler
„Datei nicht lesbar“: dann gibt mizu eine Meldung auf stderr aus und beendet sich mit Code 1
(nur wenn die Datei per CLI übergeben wurde).

---

## 15. Icons

### 15.1 Motiv
Ein Blatt Papier im Hochformat mit abgerundeten Ecken und **Eselsohr** oben rechts. Eine sanfte
**Welle** teilt das Blatt bei etwa 58 % Höhe:
- **oben** papierweiß (`#F6F5F2`) mit 3 dunkelgrauen „Textzeilen“ (abgerundete Balken)
- **unten** tiefes Wasserblau, als Verlauf `#22324F` → `#1A2438`, mit 2 hellen Textzeilen
  (`#C9D6EA`), die die oberen fortsetzen, aber invertiert

Das Motiv erzählt also „PDF + Wasser + Dunkelmodus“. Die Welle hat eine feine hellere Kante
(`#4C7BC0`, 2 % Breite) als Wasserlinie. Stil: flach, ruhig, keine Spiegelungen außer wo die
Plattform es verlangt (macOS).

### 15.2 Quellen (`assets/icons/src/`, von Hand gepflegte SVGs, Raster 1024×1024)
| Datei | Zweck |
|---|---|
| `mizu-macos.svg` | macOS: Blatt auf **Squircle**-Hintergrund nach Apples Raster (824×824 px Squircle zentriert im 1024er Canvas, Superellipse mit kontinuierlicher Krümmung, Radius ≈ 185 px). Hintergrund: sehr heller Blau-Grau-Verlauf `#EEF2F7` → `#D9E1EC`. Blatt mit weichem Schatten (y +12 px, Blur 24 px, 20 % Schwarz). Oben ein dezenter Lichtverlauf |
| `mizu.svg` | Linux und Windows (groß): **nur das Blatt**, ohne Hintergrundplatte, leichter Schatten, füllt ca. 80 % Höhe |
| `mizu-small.svg` | Windows/Linux 16–32 px: vereinfacht. Keine Textzeilen, Welle dicker, Kanten auf das Pixelraster gelegt (auf 32er Raster gezeichnet) |
| `mizu-symbolic.svg` | Linux symbolisch, 16×16, einfarbig `#2e3436`: Blatt-Umriss + Welle als Linie |

### 15.3 Generierung (`cargo xtask icons`)
- `resvg` rendert die PNGs: 16, 24, 32, 48, 64, 128, 256, 512, 1024.
  Für ≤ 32 px wird `mizu-small.svg` benutzt.
- **Linux**:
  - `generated/linux/hicolor/scalable/apps/io.github.notflask.Mizu.svg` (Kopie von `mizu.svg`)
  - `…/symbolic/apps/io.github.notflask.Mizu-symbolic.svg`
  - `…/<N>x<N>/apps/io.github.notflask.Mizu.png`
- **Windows**: `generated/windows/mizu.ico` mit 16, 20, 24, 32, 40, 48, 64, 256 (256 als PNG im ICO).
- **macOS**: `generated/macos/mizu.icns` mit allen Größen inkl. @2x (16 bis 1024).
- Die erzeugten Dateien werden **committet**, damit Packaging das xtask nicht braucht.
- Außerdem wird `generated/preview.png` erzeugt: alle Varianten nebeneinander, einmal auf hellem
  und einmal auf dunklem Grund. Das dient zur Sichtkontrolle.

### 15.4 Einbindung
- Windows: `build.rs` bettet `mizu.ico` in die `.exe` ein (nur bei `target_os = "windows"`).
  Zusätzlich gilt `#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]`.
- X11 und Windows: Zur Laufzeit wird das Fenster-Icon über `Window::set_window_icon` mit dem
  256er PNG gesetzt.
- Wayland: Das Icon kommt über `app_id` und die `.desktop`-Datei.
- macOS: `.icns` im `.app`-Bundle.

---

## 16. Packaging und Plattformen

### 16.1 NixOS (Hauptplattform) – `flake.nix`
- `devShells.default`:
  - Rust-Toolchain (aus nixpkgs oder `rust-overlay`), `rust-analyzer`, `clippy`, `rustfmt`
  - `pkg-config`, `clang` / `libclang` (für bindgen in `mupdf-sys`, `LIBCLANG_PATH` setzen)
  - `gnumake`, `python3` (falls der MuPDF-Build sie braucht)
  - Laufzeit-Bibliotheken, die per `dlopen` geladen werden, über `LD_LIBRARY_PATH`:
    `vulkan-loader`, `wayland`, `libxkbcommon`, `libGL`, `xorg.libX11`, `xorg.libXcursor`,
    `xorg.libXi`, `xorg.libXrandr`
- `packages.default`:
  - `rustPlatform.buildRustPackage` mit `cargoLock.lockFile = ./Cargo.lock`
  - `nativeBuildInputs = [ pkg-config rustPlatform.bindgenHook makeWrapper ]`
  - `postInstall`: Desktop-Datei und hicolor-Icons installieren
  - `postFixup`: `wrapProgram $out/bin/mizu --prefix LD_LIBRARY_PATH : <die Laufzeit-Libs>`
- `apps.default` → `nix run github:notflask/mizu file.pdf`
- **Prüfen**, ob der MuPDF-Build von `mupdf-sys` in der Nix-Sandbox ohne Netz läuft (die Quellen
  sind im Crate vendored). Falls er Probleme macht: Abhängigkeiten nachziehen, nicht auf
  System-MuPDF ausweichen, außer es geht nicht anders. Dann im Plan bzw. README dokumentieren.
- Desktop-Datei:
  - `Name=mizu`
  - `Exec=mizu %f`
  - `Icon=io.github.notflask.Mizu`
  - `MimeType=application/pdf;`
  - `Categories=Office;Viewer;`
  - `StartupWMClass=io.github.notflask.Mizu`

### 16.2 Andere Linux-Distributionen
Ein Release-Tarball mit Binary, `.desktop`-Datei und Icons. Ein AppImage ist optional (spätere
Aufgabe, nicht Teil der Meilensteine).

### 16.3 Windows
- Ein `.zip` mit `mizu.exe`; MuPDF ist statisch gelinkt, es gibt also keine DLLs.
- Kein Installer im MVP. In der README steht, wie man PDFs per „Öffnen mit“ mit mizu verknüpft.

### 16.4 macOS
- `.app`-Bundle: `Info.plist` mit
  - `CFBundleIdentifier=io.github.notflask.Mizu`
  - `CFBundleDocumentTypes` für `com.adobe.pdf` (Rolle Viewer)
  - `CFBundleIconFile=mizu`
- Ad-hoc signiert (`codesign -s -`), ausgeliefert als `.dmg` (`hdiutil`).
- README: Beim ersten Start Rechtsklick → Öffnen, weil die App nicht notarisiert ist.
- Datei öffnen per Finder: Das `Opened`-Event für Dateien bzw. URLs aus `winit` (macOS)
  verarbeiten.

### 16.5 CI (GitHub Actions)
- `ci.yml`, bei Push und PR:
  - Matrix `ubuntu-latest`, `windows-latest`, `macos-latest` mit `cargo fmt --check`,
    `clippy -D warnings`, `cargo test`, `cargo build --release`
  - zusätzlich ein Job `nix build` auf Ubuntu (Nix-Installer-Action)
  - Cache: `Swatinem/rust-cache`
- `release.yml`, bei Tag `v*`: Artefakte bauen (Linux-Tarball, Windows-ZIP, macOS-DMG) und an
  einen GitHub-Release hängen.

---

## 17. Meilensteine

Jeder Meilenstein endet mit einem lauffähigen Programm und grünen Checks.

| # | Inhalt | Fertig, wenn … |
|---|---|---|
| **M0** | Cargo-Projekt, `flake.nix`-devShell, `LICENSE`, CI-Grundgerüst, Fenster mit wgpu, eine Seite per MuPDF als **eine** Textur (noch ohne Kacheln) anzeigen | `nix develop -c cargo run -- tests/fixtures/sample.pdf` zeigt Seite 1 unter Wayland |
| **M1** | Layout (alle Seiten), Kamera, Scrollen mit Mausrad/Touchpad, Ziehen, Zoom zum Cursor, FitWidth/FitPage, Worker-Pool, **Kacheln**, Vorschau-Ebene, LRU-Cache, Entprellung, HiDPI/fraktionale Skalierung | Ein 300-Seiten-Skript scrollt flüssig. Bei 800 % Zoom ist der Text scharf, und während des Zoomens wird nie ein leeres Bild angezeigt. Unit-Tests für Kamera und Layout |
| **M2** | Statuszeile (glyphon + eingebettete Schrift), Keymap-Engine mit Counts und Sequenzen, alle Normal-Bindings außer Suche, Outline und Draw, Kommandozeile mit `:q`, `:<N>`, `:e` (+ Tab-Completion), Config-Datei, Session (letzte Position, Marken, Sprungliste), Drag & Drop, `app_id` | Alle Tabellen in 8.3 und 8.6 (soweit schon sinnvoll) funktionieren. Unit-Tests für Key-Parser und Keymap |
| **M3** | Dunkelmodus: Recolor-Shader, CPU-Referenz + Tests, `D`, `:dark`/`:light`, Config-Farben, Session | Umschalten erfolgt ohne Neurendern sofort, und Farben behalten ihren Farbton |
| **M4** | Draw-Modus: Stift, Live-Darstellung (lyon, MSAA), Glättung + RDP, Palette, Dicke, Cursor, Radierer (inkl. Rechtsklick), Undo/Redo, Dirty-Flag, `[+]`, E37 bei `:q` und beim Schließen | Man kann flüssig mit Maus und Stift schreiben und radieren; Undo/Redo stimmt. Tests für Eraser-Hit-Test, History und RDP |
| **M5** | Speichern/Laden: `doc/annots.rs`, `ffi_ext.rs`, `:w`, `:w <pfad>`, `:wq`, `:x`, `ZZ`, `ZQ`, atomisches Speichern, Worker entfernen mizu-Annots vor dem Rendern | Round-Trip-Test: Striche anlegen, speichern, neu laden, sind identisch (inkl. rotierter Seite). Manuell: Die Striche erscheinen korrekt in Okular/Zathura/Firefox |
| **M6** | Suche (`/`, `?`, `n`, `N`, Smartcase), Outline-Overlay, Links, Auto-Reload (inkl. Schutz bei Dirty und eigenem Save), Passwortabfrage | Funktioniert mit einem LaTeX-Workflow (`latexmk -pvc`): Die Ansicht bleibt beim Neuladen an derselben Stelle |
| **M7** | Icons (SVG-Quellen, xtask, generierte Dateien, preview.png), Einbindung pro Plattform, Desktop-Datei, Nix-Paket (`nix build`, `nix run`), Windows-ZIP, macOS-`.app`/`.dmg`, `release.yml`, README fertig | `nix run .` startet mizu mit Icon in Niri/Hyprland-Launchern. Die CI baut alle drei Plattformen |
| **M8** *(optional)* | Stiftdruck: Wenn `winit` Druck liefert (`Touch`-Events mit `force` bzw. neuere Pen-APIs), wird pro Punkt eine Breite gespeichert und als variabler Strich gezeichnet. Im PDF wird dann ein eigener Appearance-Stream (gefülltes Polygon) geschrieben, plus `/InkList` mit der mittleren Breite für fremde Viewer; die Breiten pro Punkt liegen in einem eigenen Schlüssel `/MizuW [..]` | Nur angehen, wenn M0–M7 fertig und stabil sind |

---

## 18. Tests

- **Unit-Tests**:
  - Kamera (Zoom zum Cursor ist invariant, Clamping)
  - Layout
  - Key-Parser und Keymap (Counts, Sequenzen, Overrides aus TOML)
  - Kommando-Parser
  - Recolor-CPU-Referenz
  - Glättung und RDP
  - Eraser-Hit-Test
  - History und Dirty-Flag
  - Session-Serialisierung
- **Integrationstests** (`tests/`, mit MuPDF, ohne Fenster):
  - Annotations-Round-Trip
  - rotierte Seite
  - gemischte Seitengrößen
  - fremde Annotationen bleiben beim Speichern erhalten
  - atomisches Speichern lässt bei einem Fehler das Original unverändert
- **Fixtures** werden per kleinem Testhelfer mit MuPDF erzeugt oder als winzige PDFs (< 50 KB)
  committet.
- **Manuelle Checkliste** pro Meilenstein in `docs/TESTING.md` pflegen (Wayland/Niri, Hyprland,
  HiDPI, X11-Fallback, Windows, macOS).

## 19. Performance-Ziele

- Kaltstart bis zur ersten scharfen Seite: < 300 ms für ein typisches Vorlesungsskript (≤ 20 MB).
- Scrollen und Zoomen ohne Frame-Drops bei 60 Hz und höher. Rendern blockiert nie den Hauptthread.
- Latenz beim Zeichnen: Ein Strichsegment erscheint im nächsten Frame.
- Speicher: Der Tile-Cache bleibt innerhalb des Budgets. RSS ohne Cache < 150 MB bei einem
  300-Seiten-PDF.
