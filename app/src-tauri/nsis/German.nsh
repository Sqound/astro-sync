; The bundler's messages (tauri-cli 2.11.5, tauri-bundler nsis/languages/German.nsh),
; with Astro Sync's own words for an open application: appRunningOkKill,
; appRunning and failedToKillApp. Every other line is the bundler's, kept as is,
; but for the keys `older` and `unknown`, which the bundler's German file
; misnames (`älter`, `unbekannt`): NSIS would show an empty word there, and
; for the lines the bundler writes in the formal register: the app says « du » everywhere.
LangString addOrReinstall ${LANG_GERMAN} "Komponenten hinzufügen/neu installieren"
LangString alreadyInstalled ${LANG_GERMAN} "Bereits installiert"
LangString alreadyInstalledLong ${LANG_GERMAN} "${PRODUCTNAME} ${VERSION} ist bereits installiert. Wähle den gewünschten Vorgang aus und klicke auf Weiter, um fortzufahren."
LangString appRunning ${LANG_GERMAN} "{{product_name}} ist geöffnet. Schließe es und versuche es dann erneut."
LangString appRunningOkKill ${LANG_GERMAN} "{{product_name}} ist geöffnet: Es wird geschlossen, um fortzufahren."
LangString chooseMaintenanceOption ${LANG_GERMAN} "Wähle die auszuführende Wartungsoption."
LangString choowHowToInstall ${LANG_GERMAN} "Wähle, wie du ${PRODUCTNAME} installieren möchtest."
LangString createDesktop ${LANG_GERMAN} "Desktop-Verknüpfung erstellen"
LangString dontUninstall ${LANG_GERMAN} "Nicht deinstallieren"
LangString dontUninstallDowngrade ${LANG_GERMAN} "Nicht deinstallieren (Downgrading ohne Deinstallation ist für dieses Installationsprogramm deaktiviert)"
LangString failedToKillApp ${LANG_GERMAN} "{{product_name}} konnte nicht geschlossen werden. Schließe es und versuche es dann erneut."
LangString installingWebview2 ${LANG_GERMAN} "Installiere WebView2..."
LangString newerVersionInstalled ${LANG_GERMAN} "Eine neuere Version von ${PRODUCTNAME} ist bereits installiert! Es wird nicht empfohlen, eine ältere Version zu installieren. Wenn du diese ältere Version wirklich installieren möchtest, deinstalliere besser zuerst die aktuelle Version. Wähle den gewünschten Vorgang aus und klicke auf Weiter, um fortzufahren."
LangString older ${LANG_GERMAN} "älter"
LangString olderOrUnknownVersionInstalled ${LANG_GERMAN} "Eine $R4-Version von ${PRODUCTNAME} ist auf deinem System installiert. Es wird empfohlen, die aktuelle Version vor der Installation zu deinstallieren. Wähle den gewünschten Vorgang aus und klicke auf Weiter, um fortzufahren."
LangString silentDowngrades ${LANG_GERMAN} "Downgrades sind für dieses Installationsprogramm deaktiviert, du kannst nicht mit dem Silent-Installationsprogramm fortfahren, bitte verwende stattdessen das grafische Installationsprogramm.$\n"
LangString unableToUninstall ${LANG_GERMAN} "Unable to uninstall!"
LangString uninstallApp ${LANG_GERMAN} "Deinstalliere ${PRODUCTNAME}"
LangString uninstallBeforeInstalling ${LANG_GERMAN} "Vor der Installation deinstallieren"
LangString unknown ${LANG_GERMAN} "unbekannt"
LangString webview2AbortError ${LANG_GERMAN} "Die Installation von WebView2 ist fehlgeschlagen! Die Anwendung kann ohne es nicht laufen. Versuche, das Installationsprogramm neu zu starten."
LangString webview2DownloadError ${LANG_GERMAN} "Fehler: Herunterladen von WebView2 fehlgeschlagen - $0"
LangString webview2DownloadSuccess ${LANG_GERMAN} "WebView2 Bootstrapper erfolgreich heruntergeladen"
LangString webview2Downloading ${LANG_GERMAN} "Herunterladen des WebView2 Bootstrappers..."
LangString webview2InstallError ${LANG_GERMAN} "Fehler: Die Installation von WebView2 ist mit Exit Code $1 fehlgeschlagen"
LangString webview2InstallSuccess ${LANG_GERMAN} "WebView2 erfolgreich installiert"
LangString deleteAppData ${LANG_GERMAN} "Lösche die Anwendungsdaten"
