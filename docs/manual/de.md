# TOWER Bridge Handbuch

TOWER Bridge zeigt die Messwerte des HARDWARIO TOWER Climate Module und der TOWER Sensor-Tags an, die über das TOWER Bridge Module angeschlossen sind, speichert ihren Verlauf und schaltet das TOWER Relay Module. Dieses Handbuch beschreibt Version {version}.

## Einführung {#introduction}

Sie benötigen:

- **TOWER Bridge Module**, per USB mit dem Computer verbunden.
- **TOWER Climate Module** (Revision R1 oder R2) für Temperatur, relative Luftfeuchtigkeit, Beleuchtungsstärke und Luftdruck.
- Optional TOWER Sensor-Tags, allein oder neben dem Climate Module:
  - **TOWER Humidity Tag**
  - **TOWER Temperature Tag**
  - **TOWER Lux Meter Tag**
  - **TOWER Barometer Tag**
- Optional ein **TOWER Relay Module**, ein bistabiles Relais, das Sie aus der App schalten.
- Optional ein **TOWER NFC Tag**, der einem Telefon, das ihn berührt, eine Webadresse Ihrer Wahl übergibt.
- Optional ein **TOWER CO2 Module** (Kohlendioxid), ein **TOWER VOC-LP Tag** (flüchtige organische Verbindungen) und ein **TOWER GPS Module** (Position).

Die App läuft in der Menüleiste (macOS) oder im Infobereich der Taskleiste (Windows, Linux). Sie misst auch bei geschlossenem Fenster weiter.

## Erste Schritte {#getting-started}

### Installation {#install}

- **macOS:** Öffnen Sie die DMG-Datei und ziehen Sie TOWER Bridge in den Ordner Programme.
- **Windows:** Starten Sie die EXE-Datei. Eine Installation ist nicht nötig; das Fenster nutzt WebView2, das Teil von Windows 11 ist.
- **Linux:** Machen Sie die AppImage-Datei ausführbar und starten Sie sie. Installieren Sie die udev-Regel aus dem Projekt, damit die App das Gerät ohne Administratorrechte öffnen kann.

### Hardware anschließen {#connect}

Stecken Sie das Climate Module und gegebenenfalls das Relay Module auf das Bridge Module und verbinden Sie es per USB. Die App findet die Module innerhalb weniger Sekunden. Wenn Sie Module bei laufender App wechseln, drücken Sie im Dashboard **Neu suchen**.

![So wird die Hardware angeschlossen](diagrams/hardware.svg)

Das Relay Module und die Tags sind optional, daher die gestrichelten Rahmen.

Das Bridge Module hat zwei I²C-Kanäle, und auf beiden können Sensoren sitzen, zum Beispiel das Climate Module drinnen an Kanal 0 und Tags draußen an Kanal 1. Die App sucht auf beiden Kanälen und findet einen Tag, der bei laufender App angeschlossen wird, innerhalb einer Minute.

### Menüleiste und Infobereich {#tray}

Das Symbol zeigt den aktuellen Wert der in den Einstellungen gewählten Messgröße. Sein Menü öffnet das Dashboard, listet alle Messwerte und den Verbindungsstatus auf, schaltet **Bei Anmeldung öffnen** und MQTT ein oder aus und beendet die App. Wenn Sie das Dashboard-Fenster schließen, läuft die App weiter. Wird eine Messgröße auf beiden Kanälen gemessen, zeigt das Menü beide Werte, Kanal 0 zuerst.

## Dashboard {#dashboard}

Das Dashboard zeigt das Relay Module, den letzten Wert jeder Messgröße mit Diagramm, die Karten von TOWER NFC Tag und TOWER GPS Module, wenn sie angeschlossen sind, und das TOWER Bridge Module. **Einklappen** unten in der Seitenleiste klappt sie auf ihre Symbole ein; der Pfeil an derselben Stelle klappt sie wieder aus. Ein Panel oder eine Linie hat nur, was gerade angeschlossen ist; die Messwerte eines inzwischen getrennten Moduls bleiben auf der Seite Daten. **Letzte Aktualisierung** oben gibt an, wann zuletzt gemessen wurde. Daneben legt **Intervall** fest, wie oft die Sensoren messen, von 5 Sekunden bis 3 Minuten (standardmäßig 20 Sekunden).

![Das Dashboard](images/de/dashboard.webp)

### Diagramme {#charts}

![Teile eines Diagramm-Panels](diagrams/chart-card.svg)

1. **Farbe:** wählt die Linienfarbe.
2. **Datenquelle:** Zeigen Sie auf das Symbol ⓘ, um Modul, I²C-Kanal, Adresse und Sensor der Werte zu sehen.
3. **Zurücksetzen:** kehrt zum gewählten Zeitraum zurück.
4. **Maximieren:** vergrößert das Diagramm; ein weiterer Klick oder <kbd>Esc</kbd> stellt es wieder her.
5. **Verschieben:** Ziehen Sie das Panel an eine neue Stelle; die Reihenfolge wird gespeichert.
6. **Diagramm:** Zeigen Sie darauf, um den genauen Wert und die Uhrzeit zu sehen.
7. **Min., Ø, Max.:** für den sichtbaren Bereich. Ein Klick blendet die zugehörige Linie im Diagramm ein oder aus.

Wählen Sie den Zeitraum über den Diagrammen, von 1 Stunde bis 365 Tage. Alle Diagramme einer Seite bewegen sich gemeinsam.

| Aktion | So geht es |
|---|---|
| Verschieben | Diagramm nach links oder rechts ziehen |
| Zoomen | <kbd>Strg</kbd> + Scrollen (<kbd>⌘</kbd> + Scrollen unter macOS) oder Zoomgeste auf dem Touchpad |
| Bereich vergrößern | <kbd>Umschalt</kbd> + Ziehen |
| Zurück zum Zeitraum | Zurücksetzen-Symbol (4) |

Ein roter Rand oben an einem Panel bedeutet, dass der Wert außerhalb seiner Alarmgrenzen liegt; im Diagramm ist der Bereich jenseits der Grenze hellrot hinterlegt.

Wird dieselbe Messgröße auf beiden Kanälen gemessen, etwa die Temperatur vom Climate Module und vom Humidity Tag, zeigt ihr Panel beide Werte, für jeden Kanal eine Linie in eigener Farbe und unter dem Diagramm je eine Zeile mit Min, Mittel und Max. Der farbige Punkt am Anfang einer Zeile wählt die Farbe ihrer Linie; die Spaltenköpfe blenden die Min-, Mittel- und Max-Linien ein oder aus. Ein Klick auf den Namen einer Zeile, etwa *Humidity Tag*, blendet ihre Linie im Diagramm aus; die Zeile wird grau, und ein weiterer Klick holt die Linie zurück. Eine Linie bleibt immer.

![Ein Panel mit zwei Quellen](images/de/chart.webp)

### Relay Module {#relay}

![Das Panel des Relay Module](images/de/relay.webp)

Das Relais ist bistabil und behält seine Stellung auch ohne Strom. Sein Zustand ist **TRUE** (grüne LED am Modul) oder **FALSE** (rote LED). Nach einem Stromausfall kennt die App die Stellung nicht; der Zustand ist **Unbekannt**, bis Sie das Relais einmal schalten.

![Zustände des Relais](diagrams/relay-states.svg)

- **TRUE aktivieren** und **FALSE aktivieren** schalten das Relais.
- **Impuls TRUE** und **Impuls FALSE** schalten für die **Impulsdauer** (0,5 s bis 1 Stunde, standardmäßig 2 s) in diesen Zustand und danach zurück. Die Schaltfläche füllt sich, während der Impuls läuft. Ein weiterer Klick beendet den Impuls vorzeitig.

![Ein Impuls auf TRUE](diagrams/relay-pulse.svg)

### CO₂ und VOC {#air}

CO₂- und VOC-Panels erscheinen, wenn ein TOWER CO2 Module oder ein VOC-LP Tag angeschlossen ist.

- **CO₂** in ppm. Der erste Wert kommt etwa eine Minute nach dem Start, während das Modul seinen Kondensator lädt; danach wird bei jeder Messung gemessen, höchstens alle 16 Sekunden. Der Wert ist rauschgefiltert und mit dem Barometer, falls vorhanden, auf den Luftdruck korrigiert. Der Sensor korrigiert seine Basislinie alle 8 Tage selbst, lüften Sie den Raum also ab und zu; nach einem Neustart pendeln sich die Werte innerhalb weniger Tage ein.
- **VOC** (flüchtige organische Verbindungen gesamt) in ppb, gemessen alle 30 Sekunden. Die ersten Werte kommen nach etwa fünf Minuten Aufwärmzeit. Mit einem Feuchtesensor auf demselben Kanal wird der Wert auf die Feuchte korrigiert.

Beide Sensoren behalten, was sie gelernt haben, auch wenn die App neu startet.

### GPS {#gps}

![Die GPS-Karte](images/de/gps.webp)

Die GPS-Karte erscheint, wenn ein TOWER GPS Module angeschlossen ist. Sie zeigt, ob der Empfänger eine Position bestimmt hat und wie viele Satelliten er nutzt; mit Position auch die Koordinaten, ihre Genauigkeit, die Höhe über dem Meer und die Zeit der Satelliten. Die Karte daneben funktioniert ohne Internet: Länder, Flüsse, Seen und Städte, die Position als roter Punkt. Ziehen verschiebt sie, gezoomt wird mit den Schaltflächen + und −, mit Strg/⌘ und dem Mausrad oder mit zwei Fingern; nach dem Verschieben holt eine dritte Schaltfläche die Position zurück in die Mitte. Für Straßen öffnet **Auf der Karte zeigen** die Position in OpenStreetMap. Die erste Positionsbestimmung kann einige Minuten dauern und braucht freie Sicht zum Himmel; mit einer Position leuchtet die grüne LED am Modul.

### TOWER NFC Tag {#nfc}

Der TOWER NFC Tag enthält eine Webadresse. Ein Telefon, das ihn berührt, öffnet sie, zum Beispiel eine Seite über den Raum oder das Gerät. Die Karte erscheint, wenn der Tag angeschlossen ist, und zeigt die Adresse auf dem Tag. Geben Sie eine neue ein und drücken Sie **Schreiben**; ohne `https://` wird es ergänzt. Adressen bis 240 Zeichen passen.

### TOWER Bridge Module {#bridge}

Zeigt die auf jedem I²C-Kanal erkannten Module (auch das Relay Module). **Identifizieren** lässt die LED am Bridge Module blinken. **Neu suchen** sucht erneut nach Modulen. Sind mehrere Bridge Module angeschlossen, warnt die Karte: Die App liest nur eines davon, das, dessen LED bei **Identifizieren** blinkt. Trennen Sie die anderen, sonst können sich Messwerte verschiedener Aufbauten vermischen. Die Karte steht auf dem Dashboard immer zuletzt.

## Daten {#data}

Zeigt die gespeicherten Messwerte für dieselben Zeitspannen wie das Dashboard, 1 Stunde bis 365 Tage, oder für einen eigenen Zeitraum als Tabelle, die neuesten oben. Jede Zeile mittelt einen Abschnitt, der mit dem Zeitraum wächst; wie lang er ist, steht unter der Tabelle. **CSV exportieren** und **XLSX exportieren** speichern jeden einzelnen Messwert des gewählten Zeitraums; XLSX-Dateien verwenden Ihr Datums- und Zeitformat. Exporte enthalten eine Zeile pro Messung und I²C-Kanal. Die Diagramme sind im Dashboard.

![Daten](images/de/data.webp)

## Alarme {#alerts}

Legen Sie für jede Messgröße eine untere Grenze (**Unter**) und eine obere Grenze (**Über**) fest. Verlässt ein Wert seinen Bereich, erhalten Sie eine Benachrichtigung. Die Grenzen gelten auf beiden Kanälen; wird eine Messgröße auf beiden gemessen, nennt die Benachrichtigung das Modul.

![Alarmgrenzen](images/de/alerts.webp)

- **Hysterese:** wie weit der Wert in den zulässigen Bereich zurückkehren muss, bevor der Alarm endet, damit ein Wert nahe der Grenze nicht immer wieder Alarme auslöst.
- **Wiederholen nach:** die kürzeste Zeit zwischen zwei Benachrichtigungen für dieselbe Grenze.
- **Benachrichtigen, wenn wieder normal:** meldet auch, wenn der Wert wieder im Bereich liegt.
- **Gerät getrennt:** meldet, wenn eine Minute lang keine Messung eintrifft.

![So wirkt die Hysterese](diagrams/hysteresis.svg)

Im Diagramm steigt der Wert über die Grenze: Der Alarm beginnt und Sie erhalten eine Benachrichtigung (rote Glocke). Ein Absinken in das Hystereseband beendet ihn nicht. Erst wenn der Wert unter das Band fällt, endet der Alarm; ist **Benachrichtigen, wenn wieder normal** eingeschaltet, kommt eine Benachrichtigung (grüne Glocke).

**Senden** bei Testbenachrichtigung prüft, ob Benachrichtigungen ankommen. Unter macOS bittet TOWER Bridge beim ersten Start um die Erlaubnis für Benachrichtigungen; sind sie ausgeschaltet, sagt **Senden** das, und **Einstellungen öffnen** öffnet die Stelle, an der Sie sie einschalten. Die letzten Alarme stehen unten.

## Einstellungen {#settings}

- **Allgemein:** Erscheinungsbild (System, hell, dunkel), Sprache, Datumsformat, Zeitformat, Dezimaltrennzeichen (Punkt oder Komma; „Automatisch“ richtet sich nach der Sprache), Temperatureinheit, der Wert neben dem Symbol in der Menüleiste oder im Infobereich und das Öffnen bei Anmeldung.
- **Messung:** der Luftdruck auf Meereshöhe für die Höhenschätzung und die I²C-Kanäle, auf denen nach Sensoren gesucht wird („Automatisch“ nutzt beide).
- **Daten:** wie lange Messwerte aufbewahrt werden und wo der Datenordner liegt.

### MQTT {#mqtt}

![So verbindet sich andere Software](diagrams/integrations.svg)

MQTT ist standardmäßig aus. Eingeschaltet veröffentlicht die App jeden Messwert auf Ihrem eigenen MQTT-Broker, zum Beispiel Mosquitto oder dem Broker in Home Assistant; einen eigenen Broker betreibt sie nicht. Geben Sie die **Broker-Adresse** ein und bei Bedarf Port, TLS und Anmeldung. Änderungen gelten sofort, und die App verbindet sich von selbst neu, wenn der Broker weg war. Der Status unter dem Schalter zeigt, ob sie verbunden ist.

Jeder Messwert wird als Retained-Nachricht unter dem Topic-Präfix (standardmäßig `tower`) mit dem I²C-Kanal am Ende veröffentlicht: `tower/temperature/0`, `tower/humidity/0`, `tower/illuminance/0`, `tower/pressure/0` und `tower/altitude/0`. Sensoren an Kanal 1 veröffentlichen auf `tower/temperature/1` und so weiter. Ein TOWER CO2 Module veröffentlicht auf `tower/co2/0` (ppm), ein VOC-LP Tag auf `tower/voc/0` (ppb) und ein TOWER GPS Module auf `tower/gps/0/latitude`, `/longitude`, `/altitude` und `/satellites`. Der Relaiszustand wird auf `tower/relay/0` (dem Kanal des Relay Module) als `true`, `false` oder, solange er unbekannt ist, `null` veröffentlicht. `tower/status` meldet `online`, solange die App veröffentlicht, und `offline`, wenn sie aufhört.

Mit eingeschaltetem **Relaissteuerung erlauben** nimmt das Relais `true` oder `false` auf `tower/relay/0/set` und einen Impuls auf `tower/relay/0/pulse` an.

### API und MCP {#api}

Der optionale lokale Server bietet eine REST-API, Prometheus-Metriken und einen MCP-Server für KI-Assistenten; standardmäßig ist er aus. Änderungen wie Alarmgrenzen, das Schalten des Relais oder das Beschreiben des NFC Tags erfordern **Änderungen erlauben**. Andere Computer müssen das in den Einstellungen angezeigte Zugriffstoken senden.

## Kommandozeile {#cli}

`tower-bridge-cli` ist die fensterlose Variante dieser App, für ein Terminal oder einen Rechner ohne grafische Oberfläche, etwa einen Raspberry Pi neben der Bridge. Sie teilt die Einstellungen und den Verlauf mit dieser App:

- `tower-bridge-cli` allein öffnet ein Dashboard im Terminal: die Werte mit Diagrammen, das Relais, GPS, NFC, Alarme und das Protokoll;
- `status`, `watch`, `relay`, `gps`, `nfc` und `export` geben aus, was die App zeigt, als Text oder als JSON für Skripte;
- `tower-bridge-cli run` tut, was diese App tut, nur ohne Fenster: Es speichert den Verlauf, überwacht die Alarmgrenzen, veröffentlicht per MQTT und bietet die [API](#api).

Es läuft immer nur eines davon, denn nur ein Programm kann die Bridge benutzen. Solange diese App läuft, gehen die Befehle über ihren API-Server; schalten Sie ihn deshalb unter [API und MCP](#api) ein. Um das Relais über die Kommandozeile zu schalten, muss **Änderungen erlauben** eingeschaltet sein. Die vollständige Beschreibung steht in docs/cli.md im Quellcode.

## Fehlerbehebung {#troubleshooting}

- **Gerät nicht verbunden:** Prüfen Sie das USB-Kabel. Nur ein Programm kann das Gerät gleichzeitig nutzen; beenden Sie daher andere Werkzeuge, die es belegen könnten.
- **Es läuft bereits eine andere Instanz von TOWER Bridge:** `tower-bridge-cli run` oder sein Dashboard hält die Bridge und die Daten dieser App. Beenden Sie es und starten Sie die App erneut.
- **Keine Berechtigung (Linux):** Installieren Sie die [udev-Regel](#install) und schließen Sie das Gerät neu an.
- **Keine Sensoren gefunden:** Prüfen Sie, ob Module und Tags richtig sitzen, und drücken Sie **Neu suchen**. Lassen Sie die I²C-Kanäle auf „Automatisch“ oder stellen Sie den Kanal ein, an dem sie angeschlossen sind.
- **Relaiszustand unbekannt:** Schalten Sie das Relais einmal mit TRUE aktivieren oder FALSE aktivieren.
- **Keine Benachrichtigungen:** Erlauben Sie Benachrichtigungen für TOWER Bridge in den Systemeinstellungen und verwenden Sie **Senden** bei Testbenachrichtigung.

## Über {#about}

TOWER Bridge ist Open Source unter der MIT-Lizenz, © HARDWARIO a.s. Die Dokumentation der TOWER-Plattform finden Sie unter [docs.hardwario.com/tower](https://docs.hardwario.com/tower/); die Hardware gibt es im [HARDWARIO Onlineshop](https://www.hardwario.store/).

TOWER Bridge enthält Software, Schriften, Symbole und Kartendaten anderer Urheber; ihre Lizenzen stehen unter [Lizenzen von Drittanbietern](#notices).
