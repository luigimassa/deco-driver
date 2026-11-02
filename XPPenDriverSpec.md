# XPPen Linux Driver 3.4.9-240607 — Specifiche complete di reverse engineering

> Documento generato da analisi **RE statica** (REA/Ghidra 12.1.4, Hopper-style dossier) + parsing
> statico delle risorse e dei file di configurazione inclusi nel pacchetto. **Nessuna analisi
> dinamica**: il dispositivo non è mai stato collegato durante l'analisi.
>
> Convenzione: 【V】= verificato da decompilazione/strings/symboli/binario (con indirizzo);
> 【I】= inferenza ragionevole ma non provata; 【?】= aperto.

Binario principale: `App/usr/lib/pentablet/PenTablet`
ELF x86_64, GNU/Linux 2.6.32+, 1.680.504 byte,
SHA256 `df81b8de…340f3c9`, image base 0x400000, entry 0x42b955, ~4.351 procedure ≈ 6.483 stringhe. 【V】

---

## 1. Contenuto del pacchetto e deploy

```
XPPenLinux3.4.9-240607/
├── install.sh                 (bash, root)
├── uninstall.sh               (bash)
└── App/
    ├── etc/xdg/autostart/xppentablet.desktop        Exec=/usr/lib/pentablet/PenTablet.sh /mini
    ├── lib/udev/rules.d/10-xp-pen.rules
    ├── usr/share/applications/xppentablet.desktop   Exec=/usr/lib/pentablet/PenTablet.sh
    ├── usr/share/icons/hicolor/256x256/apps/xppentablet.png
    └── usr/lib/pentablet/
        ├── PenTablet                 (ELF, Qt5 statico-ish, BSS 0x798 = 1944 B per CUSBTablet)
        ├── PenTablet.sh              (LD_LIBRARY_PATH=$dir/lib; exec $dir/PenTablet "$@")
        ├── resource.rcc              (13 MB, Qt RCC v1, 540 file)
        ├── conf/xppen/{config.xml, language.ini, name_config.ini}
        └── lib/ libQt5*.so.5 libicu*.so.56 libqxcb.so           (Qt bundled)
```

SHA256 verificate 【V】:

| File | Dimensione | SHA256 |
|--|--|--|
| PenTablet | 1.680.504 | df81b8dde7dcdeb1f992bec210bec3f287fcbb2117fd79e1c34c72aed340f3c9 |
| config.xml | 174.645 | 5121a9e41d87749fec42b05c3669384bc5d8ae3e9245f84cacd88785f2945513 |
| language.ini | 139.855 | 93f559e983ff39fd784914d52f56ac030eae7bd4d91c07f168ec6e5fcccd6aff |
| name_config.ini | 5.000 (≈) | 5c1544239c3d82367065b7213e36613eb8552e73a44f50abc87fba7c479c1e5d |
| resource.rcc | 12.964.805 | 6a70ffc040fa5f0216d40903e135a486ee7cc6138b25ee63fb81fa94f801bb61 |
| autostart .desktop | — | 3f580d2e0f9f3160a6a05a5068c988d2c2a5952a76a68f23283b07a549967398 |
| 10-xp-pen.rules | — | 8466b2cc69700f4a2af21c349b72116039b427293eb98eef871c8586ee54597e |

### 1.1 Regole udev 【V, testo completo nel file】
```
KERNEL=="uinput",MODE:="0666",OPTIONS+="static_node=uinput"
SUBSYSTEMS=="usb",ATTRS{idVendor}=="28bd",MODE:="0666"
```
→ il driver gira come **utente normale** e si affida a `/dev/uinput` 0666 + device USB 0666.
Nessun demone root nei file forniti: **non esiste alcun servizio systemd nel pacchetto**, 【V】
l'autostart avviene soltanto via `xdg autostart` con argomento `/mini` (mini-mode UI).

### 1.2 install.sh / uninstall.sh 【V, shell】
- kill di ogni `PenTablet`/`PenTablet.sh` running (`ps -e | grep` + `kill -15`).
- `cp` udev rule → `/lib/udev/rules.d/`, app → `/usr/lib/pentablet`, shortcut/i/autostart.
- permessi finali: binario e .sh `0555`; `conf/xppen/{config.xml,language.ini,name_config.ini}`, `resource.rcc` e
  `conf/xppen/` **0777/0666 → scrittura da parte dell'utente** (asset di funzionalità: config.xml scrivibile a livello globale).
- crea `/tmp/qtsingleapp-Pentab-9c9b-lockfile` `0666` (mixato con CSingleApp Ri sotto).
- `uninstall.sh` rimuove le stesse cose + `rm /tmp/qtsingleapp-Pentab-9c9b-lockfile`.

### 1.3 Argomenta CLI 【V da strings/uso】
`PenTablet [/mini]` — mostra finestra "mini" (tray-driven). 【V da .desktop autostart】
`PenTablet.sh` passa argomenti invariati.

---

## 2. Architettura: processi, thread, signal/slot wiring 【V】

### 2.1 Chi avvia cosa

`MainWindow::Init` @0x486770 【V】:
```
EnmuTabeltDevice()              // enum + apertura dei device (OpenDeviceLinux su ogni match)
MyInput::Init(CGlobalData::MyInputDriver)   // inizializza il layer di input
UsbListenThread::create_listener()          // su QThread nuovo (0x28 byte specific alloc)
connect(usbThread,  SIGNAL(SentListenMsg(unsigned short, unsigned short, short)),
        this,        SLOT(ReceiveUsbMsg(unsigned short, unsigned short, short)), 0)
```
Il driver è **un solo processo Qt UI che fa anche da parser/injector**: i thread sono
`UsbListenThread`, `DeviceThread`, e il main event loop Qt; l'iniezione input e la UI
condividono lo stesso indirizzo PID. 【V: nessun exec/fork di altri binari nelle funzioni analizzate】
L'install.sh tocca solo file e lockfile; nessun demone root nel pacchetto. 【I — l'ipotesi di
"due processi" dei draft precedenti è scartata: visione corretta = mono-processo + thread interni】

### 2.2 Thread enum/entry 【V】

| Thread/oggetto | indirizzo | Ruolo |
|--|--|--|
| `UsbListenThread::create_listener` | 0x42fe40 | `libusb_init`, `libusb_hotplug_register_callback(ctx, events=3 (ARRIVED|LEFT?), flags=0, VID=0x28bd, PID=<any>, class=<any>, cb=libusb_callback, usr=this)` |
| `UsbListenThread::run` | 0x430100 | `while (!this[0x10]) libusb_handle_events_timeout(ctx, {0, 500000})` (tick 0.5 s) → `delete_listener` |
| `UsbListenThread::libusb_callback` | 0x4300d0 | chiama `tablet_callback(this, ctx, dev, event)` e ritorna 0 |
| `UsbListenThread::tablet_callback` | 0x42ff40 | `libusb_get_device_descriptor` → `sprintf("VENDOR(0x%x) PRODUCT(0x%x)")` (log) → `SentListenMsg(vid, pid, event)` (signal) |
| `UsbListenThread::SentListenMsg` | 0x504f90 | signal Qt(quint16 vid, quint16 pid, short event) |
| `DeviceThread::run` | 0x4c56c0 | reader loop HID, vedere §3.2 |
| `DeviceThread::MySentTabletPackets(uchar*, int)` | 0x50a710 | `QMetaObject::activate(..., idx 0, {uchar*, int})` — signal con payload raw |
| `DeviceThread::LinuxRelease` | 0x4c5d20 | `hid_write(h, {02 00 2B 00…}, 10)` su uscita + zero contatore errori |
| `DeviceThread::HidOperatorError` | — | slot per disconnessione 【usi: v.	NdrFcer init legami】 |

### 2.3 Collegamenti signal/slot per device 【V — in `CUSBTablet::InitTablet` 0x4380c0】
```
DeviceThread nuovo (0x20 byte) →
  connect(dThread, "2MySentTabletPackets(quint8*,int)", cusb, "1ReceiveTabletPackets(quint8*,int)", 1 /*Direct*/);
  connect(dThread, "2HidOperatorError(int)",            cusb, "1OnDeviceConnError(int)", 1 /*Direct*/);
  QThread::start(dThread, 7);           // priority 7
```
Con connessione **Direct** = slot corre nel thread lettore. 【V: connect flag `1` = Qt::DirectConnection】

### 2.4 Modello "famiglia" multidevice 【V da InitTablet】
`CGlobalData::TabletArray` — array di tavolette (una per dispositivo collegato). Su re-enum di un
device duplicato l'intero blocco istanza (almeno `0x3EE98` byte, 257.936) viene **clonato con memcpy**
dall'istanza precedente + `operator_new(0x3EDF8)` memset(0) per lo spazio dati; flag `entry+0x5A`
segnala "config già presente". In `InitTablet` viene ricostruita inoltre una
`QMap<QString, _stAppConfigNode*>` per-app.

### 2.5 Bootstrap `main()` @0x4273E0 【V】
```
1. QCoreApplication::setAttribute(AA_UseDesktopOpenGL=0xD, true)
2. new QtSingleApplication("PentableV3", argc, argv)
     → se isRunning(): sendMessage("Please show PentableV3") e exit
       (lockfile /tmp/qtsingleapp-Pentab-9c9b-lockfile)
3. QResource::registerResource(appDirPath() + "/resource.rcc")
4. QApplication::setStyleSheet(read(":/qss/qss/stylesheet.qss"))   (2173 righe QSS)
5. FONT globale: QFont("Arial")
6. new MainWindow ; connect(QtSingleApplication::messageReceived → MainWindow::OnShowDriver)
   (la seconda istanza, tramite QtSingleApplication, "riattiva" la finestra esistente)
7. se argv[1] == "/mini" → InitSystemTray() + hide()
   altrimenti show()
8. MainWindow::Init() → exec() loop Qt
```
Tray: QSystemTrayIcon + TrayMenu (QMenu 0x50) con azioni showAction → `OnShowDriver`,
updateAction → `OnUpdateApp`, quitAction → `OnQuiqDriver`; icona `:/image/Res/pentablet.png`;
tooltip da literal; `activated` → `onActivated(ActivationReason)`.
`MainWindow::ReceiveUsbMsg(vid, pid, event)` @0x486840:
event==1 (attach) → **`usleep(600000)`** poi `EnumDeviceLinux(pid)`; se il combo result ha
1 entry → `EnumDisplay` + `InitUI(text)`; event==2 (detach) → rimozione entry da `TabletArray`
+ cleanup UI/DeleteMapItem.

---

## 3. APC HCIUSB: protocollo verso il dispositivo  【tutti i dati = V da disassembling】

Il canale di scrittura device è `hid_write` su report **ID 0x02**; il canale di lettura è
`hid_read_timeout` con buffer 0x40 (64) e timeout 100 ms.
Il sink device è `*(long*)(this+0x788)` (handle hidapi di `CUSBTablet`).

### 3.1 `CUSBTablet::SentCmd(int type, int v1, int v2)` @0x4371e0

| type | report (byte[]) | len | note |
|--|--|--|--|
| 0 | `02 B4 01 v1 …` | 12 | richiede flag `this[+0x61b] & 0x18 != 0`; handle != 0 |
| 1 | `02 B4 01 01 v1 v2` | 12 | |
| 2 | `02 B4 02 01 v1 (v2 se v1==1)` | 12 | |
| 3 | `02 B0 v1 …` | 10 | |
| 0x6E | `02 B5 00 F0 …` | 10 | richiede **firmware int > 0x959** e `this+0x5F4 == 2` |
| 0x6F | `02 B5 01 01 00 v1 v2` | 10 | idem gate |
| 0x70 | `02 B5 01 02 00 v1 v2` | 10 | idem |
| 0x71 | `02 B5 01 03 01 01 02 v1 v2` | 10 | idem |
| 0x72 | `02 B5 01 03 01 02 v1 v2` | 10 | idem |
| 0x73 | `02 B5 01 03 01 01 03 v1 v2` | 10 | idem |
| 0x75 | `02 B5 03 00 00 v1 v2` | 10 | idem |
| 0x76 | `02 B5 04 00 00 v1 v2` | 10 | idem |
| 0x77 | `02 B5 07 00 00 00 v1` | 10 | idem |

Nota: per i tipi 0x6E..0x77 il "gate firmware" è calcolato a runtime dalla stringa firmware
letta in `InitTablet` da `hid_get_indexed_string(h, 0x6E, this+0x618)` → `toInt()`;
solo firmware numerico > 0x959 (2393) e `this+0x5F4 == 2` (PenType 2) accettano i comandi `B5`. 【V】

### 3.2 Loop di lettura — `DeviceThread::run` @0x4c56c0 【V】
```c
do {
  while (!isInterruptionRequested()) {
    memset(buf64, 0);                       // buf 64 byte
    n = hid_read_timeout(h, buf, 0x40, 100);
    if (n > 0) { MySentTabletPackets(buf, n);   // emit  →  CUSBTablet::ReceiveTabletPackets
                 qCLog("default", "src recv %02X %02X …"); }
    else if (n == 0) continue;              // timeout: ricomincia
    else break;                             // errore
  }
  errCount = *(this+0x18) + 1;              // inc contatore fallimenti
  usleep(100);
} while (errCount < 0x9c4);                 // 2500 tentativi consecutivi
HidOperatorError(this, n);                  // segnala disconnessione
qCLog "default" + hid_error(h); usleep(50);
```

### 3.3 `SentSaveFileCmd(int packetId, uchar* data, int len)` @0x436e30 【V】
```
base = {02 B6 packetId …}
packetId == 2  → len=0x20 (32 byte fisso)
packetId == 1  → bytes[3..6] = packLE32(len), poi scrittura completa di `len` byte
packetId == 3  → cpu appende `data` (len+3) subito dopo l'header
pointer == 0   → (none)
```

### 3.4 `SentReadFileCmd(int packetId, int arg)` @0x436ee0 【V】
```
base = {02 B6 …}; len 10
packetId == 2 → bytes[3] = arg ? arg : 2
packetId == 1 → bytes[3] = 1, `this[0x58]=0`
other         → bytes[2] = 2
```

### 3.5 `DeviceThread::LinuxRelease` @0x4c5d20 【V】
uscita dal thread: `hid_write(h, {02 00 2B 00 …}, 10)` + `*(this+0x18) = 0`.

### 3.6 Inizializzazione device — `CUSBTablet::InitTablet(path, UgeeDevice&)` @0x4380c0 【V】

Ordine delle operazioni (estratto dai renti):

1. `hid_open_path(path)` → handle a `this+0x788`.
2. Read capability **dalla USB index string 100**:
   - `hid_get_indexed_string(h, 100, buf, 0x40)`
   - Valori decodificati (& contatoriali in `sprintf("manufacturer:%s product:%s Get Max X:0x%x Y:0x%x Press:0x%x\n")`):
     - `this+0x5F8` = MaxX = `b0 + b1*0x100 + b?*0x10000` (24 bit)
     - `this+0x600` = MaxY = idem dai byte successivi (24 bit)
     - `this+0x614` = PressMAX = `b6 + b7*0x100` (16 bit)
     - `this+0x608` = secondo valore 16 bit b`?`*0x100+b`?`, default **0x13D8** (= 5080) se zero.
3. `hid_get_indexed_string(h, 0x6E, this+0x618, 0x40)` — **firmware/product serial string**;
   byte `this[0x619]` checked `== 0x40` per le scelte di path del QString di presentazione.
4. `sprintf(this+0x460, "PID_%04X", pid)` — chiave per **`name_config.ini`**; device display name:
   - Default letto da `QSettings(CGlobalData::Name_Config)` con quella key → `this+0x511`;
   - se vuoto → `strcpyp(param_2 + 0x14, this+0x511)` (nome interno `UgeeDevice`).
5. Copia stato/risorse per device già presente (§2.4).
6. **Caso speciale** 【V — solo se】 `(PenType==4 && PID==0x201)` (il dongle "AC19" nel name_config
  .containsKey 0x201) → `hid_write(h, {02 ?…}, 10)`.
7. Avvio `DeviceThread` + doppio invio di configurazione iniziale:
   - QMutex::lock() → `vtable+0x68(this, 0, 1, 0)` = `SentCmd(0,1,0)` → `02 B4 01 01`
   - QMutex::unlock() → `vtable+0x68(this, 0, 2, 0)` = `SentCmd(0,2,0)` → `02 B4 01 02`

### 3.7 Ciclo di gestione eventuale disconnessione 【V】
`CUSBTablet::ReopenTablet` @0x437e40: `SendDeviceChangeMsg(0x28bd, pid, 2, path)` → `usleep(200000)`
→ `SendDeviceChangeMsg(0x28bd, pid, 1, path)`. Il "subtype" 2/1 sono rispettivamente
detach/attach 【I naming】. `SendDeviceChangeMsg` @0x507000 è signal Qt con 4 argomenti.

---

## 4. Packet input parsing — `CParseTabletData` 【V da decompilazione completa】

### 4.0 Struttura `TabletDataEX` (offsets verificati sul binario) 【V】
```
+0x04  int    PID del device corrente
+0x08  ushort FLAGS (bit: 1=pen, 2=keyboard, 4=ring/frame, 8=pen-buttons, 0x20=no-report)
+0x10  long   X          (24 bit: buf[2]+buf[3]*0x100, +buf[10]*0x10000 se pacchetto lungo)
+0x18  long   Y          (idem,buf[4..5], +buf[11]*0x10000)
+0x20  uint   Pressure   (buf[6]+(buf[7]&0x1F)*0x100; 19-bit se len>=0xE: +buf[0xD]&7)*0x2000)
+0x24  byte   TiltRX     (buf[8], solo pacchetti lunghi)
+0x25  byte   TiltRY     (buf[9], idem)
+0x26  byte   raw byte1  (stato tipa pen)
+0x27  byte   penBtn changed-mask
+0x28  byte   penBtn pressed
+0x29  byte   mode/group 0..3 (cache this[0x32] se nonzero, altrimenti buf[7]>>5)
+0x30  TabletKeyData  (0x50 = 2×0x28: stato corrente + history, 12 tasti)
+0x68  ushort lnum direzione frame
+0x6A  ushort rnum direzione frame
+0x78  long   ptr stTablet_XMLData corrente
+0x80  ushort lnum raw (dal pacchetto)
+0x82  ushort rnum raw (dal pacchetto)
```

### 4.1 `CParseTabletData::ParseData(buf, len, TabletDataEX*)` @0x4C63C0 — albero di dispatch 【V】
```
buf[0] == 0x06
  → ChangeDataToKeyState(buf, keyStruct) ; FLAGS |= 2 (keyboard frame)
buf[1] == 0xF6
  → probeReport = ((buf[2]&0xF0) + buf[3]*0x100) >> 4
      0x501 → this[0x32] = 2   (modalità device 2)
      0x801 → this[0x32] = 3   (modalità device 3)
      0x301 → this[0x32] = 1   (modalità device 1)
      return 1
buf[1] == 0xC0  oppure  (buf[1]&0x30)==0x20
  → BLOCCO PEN:
      ChangeToPenBtnState(buf[1]&0xF) → FLAGS|8, aggiorna +0x27/+0x28
      FLAGS|1 ; X/Y come in 4.0 (pdo len<0xC short, >=0xC long 24-bit)
      Pressure/Tilt per len (4.0) ; pen state raw → +0x26
      mode → buf[7]>>5 se this[0x32]==0
altrimenti
  → BLOCCO KEYBOARD/FRAME (sottotipi buf[1]):
      1  → FLAGS|4,  lnum=0,  rnum=1        (PID 0x202 → ChangeK05DataToKeyState)
      2  →          rnum=0xFFFF
      4  →          lnum=1,  rnum=1         (PID limite 0x96/0x74 → ChangeDataToKeyState)
      8  →          lnum=1,  rnum=0xFFFF
      0x10→          lnum=1,  rnum=1         (variante di quei PID)
      0x20→          lnum=1,  rnum=0xFFFF
      0x42/0x80→ chave ChangeDataToKeyState (PID 0x96/0x74/0x78)
      0xFF→          rnum=0xFFFF
      default: FLAGS|4, lnum=0, rnum=1
```
Le funzioni di conversione chiave restano confermate e complete:
- `ChangeToKeyBoard` @0x4C5E30: riempie `TabletKeyData` (struct 0x50) — 12 slot tasto
  (`keyState[0..11]` da `buf[3..0xE]`, cache history in +0x28) 【V】.
- `ChangeDataToKeyState` @0x4C5F70: buf[0]==6 → bit 0..31 dai byte [1..4]; altrimenti 40 bit da
  [2..6]. Se bit cambiato → `keyState[i]=1/0`; cache à +0x28.
- `ChangeK05DataToKeyState` @0x4C6150: variante K05 (ACK05/ShortcutRemote), gruppi K0..K3 【V】.
- `ChangeToPenBtnState` @0x4C6340: 4 bit → PenBtnData 【V】.

**Non esiste una funzione `ChangeToTouchData` nel binario**: il parsing touch è gestito dentro
`ParseData` via i sottotipi keyboard/lnum/rnum (ring/touchpad convertiti ad assi） 【V — confermato da
inventario completo delle procedure】.

### 4.2 `CTablet::ReceiveTabletPackets(buf, len)` @0x4C75D0 — dispatcher report-id 【V】
Questo è lo smistatore dei **report di risposta** che NON sono dati penna. Attivi:
```
buf[1]==0xB4  → ACK comandi luce:
      buf[2]==1: buf[4]==2 → this+0x658 = buf[5] e buf[4]==1 → this+0x65A = buf[5]
                 in entrambi i casi wakeAll(cond read) + SentTabletLightData (echo brightness)
      (var. buf[3]...)
buf[1]==0xF2  → this+0x1C = buf[3]; PostActionMsg(this+0x76C, 0x58, buf[4])
                 (evento asincrono dal device → internal action idx 88 【? semantica specifica】)
buf[1]==0x06  → risposta capability/config:
      subtype buf[2]==0x64 'd': ritorna MaxX (24 bit), MaxY, Press alla vecchia ricezione;
               recalc percentuali vs 0x13D8; wakeAll
      subtype buf[2]==4  : copia blocco config (fin 0x38 byte) dalla risposta greadata
               a this+0x549 (check buf[3]==1)
buf[1]==0xF8  → if (buf[2]==2 && buf[3]==1) → SentCmd(3, ...) e, per PID this+0x768 ∈ {0x933,0x934}
                (Deco Pro SW/MW), anche SentCmd(0,1)+SentCmd(0,2)+ReceivActionMsg(100,0)
                handshake extra 【V】
buf[1]==0xB0  → wakeAll (ACK della SentCmd type 3)
buf[1]==0xB6  → pipeline Trasferimento FILE (config backup su device):
      buf[2]==1,buf[3]==1 → this[0x59]=1,                 PostDeviceOperatorMsg(1,0) START
      buf[2]==2,buf[3]==2 → this[0x59]=0,                 PostDeviceOperatorMsg(3,2) END
      buf[2]==0,buf[3]==1 → this[0x58]=1 (READ)  → total = buf[?]
                 total==0 → Post(10,0) | this+0x54==buf[8] → Post(5,total) | altro → Post(9,0)
      buf[2]==2,buf[3]==1 && this[0x58] → append dati al QByteArray buffer (this+0x48,
                 capienza base 0x3EDF8), progress Post(6, %
                 completamento: XMLHelp::ReadDeviceConfigFileToAppConfig(...), verifica
                 campi validi (short >= 0) a +0x3EDEA e +0xB788 → Post(7,1) ok | Post(8,1/0) err
      buf[3]==3 → ReceivActionMsg(pid, 0x75, buf[6]); buf[3]==4 → ReceivActionMsg(pid, 0x76, buf[6])
      buf[3]==1 (buf[4]==1/2) → ReceivActionMsg(0x6F/0x70, buf[6]) config extra
```
Dati penna/report-frame passano poi in `CParseTabletData::ParseData` e, se il frame ha Azione,
in `CAction::ActionTabletData` (§5.3).

---

## 5. Action layer `CAction` 【V — decompilazione completa】

### 5.0 Mappatura ID azione builtin → handler (switch di `ActionLibrary` 0x4CAC90) 【V】
| ID azione | handler | effetto | PostActionMsg |
|--|--|--|--|
| 1..22 (0x01–0x16) | sequenze inline | combinazioni keyboard (B/E/Alt/Space/Ctrl+…) via `KeyEvent`/`KeyEvents` (struct `_KeyCode` stride 0x10, `SleepFuctions(4)` tra i tasti) | msg 99 |
| 0x17..0x1A (23..26), 101 (0x65) | `ShowDriver` | log "ShowDriver No Implement" …; se pressed → PostActionMsg(1) | 1 |
| 102 (0x66) | `ChangeScreenMapping` | if pressed → PostActionMsg(2) | 2 |
| 103 (0x67) | `ChangeBEKey` | toggle pen/eraser; PostActionMsg(3, v) + `MyLrc::EmitInfo(Show_Info, "Pen"/"[B]")` OSD | 3 |
| 104 (0x68) | `OpenFineMode` | log "FineMode"; PostActionMsg(4) | 4 |
| 0x69/0x6A/0x6B (105..107) | `ChangeTrackPadMode` | PostActionMsg(7) (R1/R2/touch touchpad) | 7 |
| 109 (0x6D) | `PressBEKey` | singolo colpo B o E (0x30=KEY_B / 0x12=KEY_E) dallo stato corrente this[0x72] + label OSD lato lingua | – |
| 111 (0x6F) | `KeypadPreviewMode` | apre preview tastierino (dati stringa this+0x78) | 11 |
| 112 (0x70) | `ChangeKeyFunctions` | cicla **gruppo tasti** 0..3 leggendo enable flag `appConfig+0x168E2+g*0xA142` e aggiornando `xml+0x3EDEA` → PostActionMsg(10, …) | 10 |
| 199/200, 0xD4/0xD5 | (return) | no-op Linux (conferma switch) | – |
| 201..211 | `MouserActionLibrary` | forwarding diretto a `MyInput::DoMouseFunc(id, pressed, extra, false)` | – |
| custom (RunApp / KeySeq) | `ParseCustomFunction` | see §5.5 | – |
| default | (fallback) | `PostActionMsg(99, "DoKeyAction"+num)` per feedback UI | 99 |

Sottotipo **subtypes PostActionMsg** registrati 【V】: 1=show panel, 2=screen mapping, 3=BE mode,
4=fine mode, 7=trackpad, 10=key group change, 11=preview, 99=UI feedback evento hotkey
(`"DoKeyAction"/"DoPenBtnAction"/"DoRingAction CustomFun"` + valore).

### 5.1 `CAction::DoAction(TabletDataEX&, stTablet_XMLData*)` @0x4D5C40 【V】
```c
app = *(long*)(this+0x78);                  // della FindForeAppConfig (§5.3)
this[0x88] = (*(short*)(xml + 0x7E) != 0)   // flag "TouchRing relativo" o simile
if (raw pen state (TDataEX+0x26)+0x58 < 2):
   this[0x73] = 1                            // "pen in-range"
   this[0x74] = (*(short*)(app + 0x408 + (byte)TDataEX[0x29]*0x1470 + 0x20) == 0)
if (app flag byte (app+0x408+idx*0x1470) != 0)
   → MyInput::UpdateRelMouse(mode_byte, dx=this+0x2C, dy=this+0x2E)
     (dx/dy se in-app-disabled flag → passati 0)
if (this[0x73]==0 || this[0x74]==0):
   MyInput::UpdateDigit(mode_byte, X=this+0x48, Y=this+0x50, pressure=this+0x40,
                         keystate=this+0x40, tiltX=this[0x60], tiltY=this[0x61],
                         /*pressed*/ CVar6)
else → digits azzerati (hover-only: disableInApp)
if (flags&8) → this[0xB4]=1 ; DoPenBtnAction(mode)
if (xml + 0x80 == 0) {                       // ring not relative
   if (flags&2) → DoKeyAction(mode, (xml+0x2A) != 0)
   if (flags&4) → DoRingAction()
}
```
`this+0x2C/0x32/0x40/0x48/0x50` sono campi calcolati in `ParsePenDataLinux` (rotazioni/mirror,
scala risoluzione da `xml+0x598/0x5A0/0x5AC/0x5B0`, `xml+0x44`/`xml+0x48`%).

### 5.2 `ParsePenDataLinux(TabletDataEX&, _DEVICE_ATTRIBUTES&, stTablet_XMLData*)` @0x4CB470 【V】
Gestisce **rotazioni** in base a `xml+0x48` (Orientation):
```
0 → X=xml+0x5A0-dir, Y=xml+0x598 (normale)
1 → swap X/Y e negazione assi prevX/prevY (ruoto 90°/270°)
2 → mirror entrambi assi (180°)
3 → mirror "misto" (+/-1)   (commutato sul TiltX/TiltY signs in _DEVICE_ATTRIBUTES+0x24/25)
→ poi scala float skill/scale (steps of resolution conversion) da xml+0x44..0x4A per output
```
`ParsePenData` e `ParsePenDataLinuxV1` @0x4CC320/0x4CC1F0 sono **duplicati identici** dello stesso codice 【V】.

### 5.3 `FindForeAppConfig` @0x4CAC50 【V】
Nel binario Linux è un semplice assegnamento: `xml+0x78 = map.value(...)` — seleziona il nodo
per-app attivo nella QMap `QMap<QString,_stAppConfigNode*>`; **nessuna** identificazione reale
della finestra foreground (quello è supervisione nella UI o stub). Nodo per-app layout verificato:
`+0x408` enable flag, `+0x428+idx*0x1470` per-key slot, `+0x1470` stride, `+0x168E2+g*0xA142`
flag gruppo, `+0x3EDEA` (gruppo corrente come short), `+0x16CE8` `Actid` del tasto corrente.

### 5.4 Ring `CAction::RingActionLibrary(mode, dir, extra)` @0x4CB1F0 【V】
```
mode 1/2  → inversione direzione (dir = 3-dir / 2-dir); se dir==0 → MouserActionLibrary(1)
            (wheel scroll; celebrativo se extra==1 o -1)
mode 0    → ActionLibrary(dir? press(1), SleepFuctions(5), release) — "pagina/giù su stack"
mode 2    → ActionLibrary(press/5ms/release)
mode 3    → KeyEvent(0x38=ALT …, 0xB7/0xB8, up/down) — combo Alt+Key
```
`ParseRingCustomFunction` @0x4CD850 parse della stringa custom ring:
`split('|')` → `[0]` type (toShort), `[2]` o `[3]` per direzione (`-1` = left); poi
`split(',')` → `split('+')` (combinazioni) → `split(':')` (`code:stato`) → `KeyEvent(code, stato, pressed)` 【V】.

### 5.5 Mini-linguaggio "custom function" (config keys/pen) 【V — ParseCustomFunction 0x4CFD40】
Formato stringa salvato in config.xml:
- **3 segmenti**: `mode|name|data` per mode 0 (keysequence), 2 (run app), 3 (azione builtin)
- **4 segmenti**: `mode|name|?|data` per mode 1 (app-launch con metadati extra)
- Gli id del mode 3 si riferiscono alla tabella azioni builtin (es. XML `Actid="207"`,
  `Motid` per mouse). Se il payload è una lista tasti, viene decomposto come
  `split(',')` → `split('+')` → `split(':')` e inoltrato a `KeyEvents` (struct `_KeyCode`,
  stride 0x10, `SleepFuctions(4)` tra i tasti, 10 ms tra pressioni ripetute 【V】).

### 5.6 `ParseKeyData` @0x4CCCF0 / `ActionCorrectorData` @0x4CD370 【V】
- `ParseKeyData`: confronta i 12 slot `TabletDataEX+0x30` con la history `+0x30+0x28`;
  per ogni tasto cambiato → `GetKeyIndex(id)` (ricerca nella tabella per-app
  `xml+group*0xA142+0x16D82`, 0x28 slot × stride 0x408, oppure globale `xml+0xCC3E`);
  correzione rail-key: se `idx >= KeyNum - (1 o 2 in base a xml+0x2C)` → `idx += xml+0x30`
  (offset tastiera dispositivi con tasti extra). Contrazione in QVector<short> `this+0xA0`.
- `ActionCorrectorData`: riproduce l'elenco accumulato (`ActionLibrary(idx, pressed)` con
  10 ms di sleep; se idx==10/11 → `DoMouseFunc(5/6)` = scroll wheel invece del tasto).

### 5.7 Ancillari
- `run_shell(char*)` @0x4CC330 — **`popen(cmd,"r")` + read + `pclose` reale** 【V】
  (esecuzione user-space senza elevate).
- `SleepFuctions(ms)` @0x4CB1E0 = `usleep(ms*1000)`.
- `KeyEvent` @0x4CA8A0 → `MyInput::KeyEvent(CGlobalData::MyInputDriver, code, pressed)`;
  `MacKeyEvent`/`PenBtnActionLibrary`/`InitInput` sono **stub vuoti** nel binario Linux 【V】.

---

## 6. Iniezione di input — `MyInput` 【V — decompilazione completa】

`MyInput::Init` crea **tre dispositivi uinput** separati:
| handle | nome | tipo | id |
|--|--|--|--|
| `this+0` | `XP-Pen Pen` | EV_ABS digitizer | bus=3(BUS_USB), vendor=0x28BD, product=2, ver=1 |
| `this+4` | `XP-Pen Mouse` | EV_KEY+EV_REL | bus=3, vendor=0x28BD, product=0x22, ver=1 |
| `this+8` | `XP-Pen Eraser` | EV_ABS digitizer | bus=3, vendor=0x28BD, product=2, ver=1 |

### 6.1 Ioctl uinput edificio (valori esatti) 【V】
```
UI_GET_VERSION   0x8004552D   (if version > 4 → path moderno)
UI_ABS_SETUP     0x401C5504   input: {code, value, min, max, fuzz, flat, resolution}
UI_DEV_SETUP     0x405C5503   input: struct uinput_setup {id, name[80], ff_effects_max}
UI_SET_EVBIT     0x40045564
UI_SET_KEYBIT    0x40045565
UI_SET_RELBIT    0x40045566
UI_SET_ABSBIT    0x40045567
UI_DEV_CREATE    0x5501   (fallback legacy: write(uinput_user_dev) 0x45C bytes)
```
FALLOFF legacy: se versione uinput <= 4 si scrive `uinput_user_dev` (0x45C) con absmax[] array.

### 6.2 `XP-Pen Pen` (abs digitizer) setup esatto 【V】
```
EV: EV_SYN(0), EV_KEY(1), EV_ABS(3)
ABS_X         max 0xCC64 (52324), resolution 200     ← default; ricalcolato dal device max
ABS_Y         max 0x73A0 (29600), resolution 200
ABS_PRESSURE  max 0x1FFF (8191)
ABS_WHEEL (8) max 0x47 (71)
ABS_TILT_X/Y  min -63 … max 64 (0xFFFFFFC1…0x40)
KEY: 0x140 BTN_TOOL_PEN, 0x141 BTN_TOOL_RUBBER, 0x142 BTN_TOOL_BRUSH,
     0x14A BTN_STYLUS, 0x14B BTN_STYLUS2, 0x110/0x111/0x112 (mouse 3 btn), 0xB1/0xB2
```

### 6.3 `XP-Pen Mouse` (rel) 【V】
```
EV: EV_SYN, EV_KEY, EV_REL
REL: REL_X(0), REL_Y(1), REL_WHEEL(8)
KEY: 1..0x100 (layout keyboard), 0x110/0x111/0x112
```

### 6.4 `XP-Pen Eraser` (abs) 【V】
```
stessi ABS di Pen, KEY: 0x141 BTN_TOOL_RUBBER, 0x14A/0x14B BTN_STYLUS*
```
Serve perché il driver mappa la **gomma/eraser tool** su device separato (tool-switch
fisico, disturnato comodo per compositor).

### 6.5 Update paths 【V — tutti verificati】
```
UpdateDigit(state, x, y, press, key, tiltX, tiltY, pressed)  → evento EV_ABS completo + SYN su abs/eraser
UpdateRelMouse(state, dx, dy)                                → EV_REL + mask BTN su rel device
UpdateWheelRel(fd, delta)  → 2 eventi: EV_REL REL_WHEEL delta + SYN
KeyEvent(code, pressed)    → EV_KEY + SYN (usleep(100) tra scritture)
DoMouseFunc(btn 1..6):
   1 left   → EV_KEY 0x110 press+release
   2 right  → 0x111,  3 middle → 0x112 (+0x14B/0x14C BTN_STYLUS accoppiati)
   4        → doppio click (2× left con usleep 20 ms)
   5/6      → wheel scroll su/giù (delta = ±1)
MyInput::ActionLibrary(id 1..10) → gateway per K05 (ACK05) builtin veloci:
   1=0x38(LALT) 2=0x39(RALT) 3=0x83(RCTRL) 4=0xB6 5=0x30(B) 6=0x12(E)
   7=CTRL(0x1D)+0x4E  8=CTRL+0x0C(MINUS)  9=CTRL+0x2E(C)  10=CTRL+0x2F(V)
MyInput::RingActionLibrary(id 0..4) → gateway ring veloce: MINUS/EQUALS(+CTRL), PAGEDOWN/PAGEUP, ALT combos
```
Il main **dispatcher** in uscita resta:
```
CAction::DoAction → MyInput::UpdateDigit / UpdateRelMouse / UpdateWheelRel / KeyEvent
CAction::MouserActionLibrary → MyInput::DoMouseFunc
```
`MyInput::Init` viene invocato da `MainWindow::Init` prima del device open (Create device anche
nessuno tavoletta collegata = device uinput sempre presenti finché il driver gira).

---

## 7. Persistenza configurazione 【V/File-verified】

| File | Formato | Uso 【V】 |
|--|--|--|
| `/usr/lib/pentablet/conf/xppen/config.xml` | XML `PenTableLists version=1.5.0` | database **categorie/device**: per ogni modello DeviceInfo + Common (ScreenMapping/TabletPannel/Disable*/PictureFiles) + CommonAPP (Pen/K/R) + 53 nodi `app` per per-app override |
| `/usr/lib/pentablet/conf/xppen/language.ini` | INI, 22 lingue (≈495 key) | UI copy |
| `/usr/lib/pentablet/conf/xppen/name_config.ini` | INI, `[Common]`+`[Dongle]`+`[CopyRight]` | da **PID USB → nome marketing device** (60+ PIDs) |
| user data | `<home>/AppData/Roaming/Pentablet V3/data/mymac.ini` | persistent per device-instance config 【path inherited da Windows code; usato anche qui】 |
| Qt settings | `CGlobalData::Language_Config` e `Name_Config` (QSettings) | lookup nome device e lingua |

XML help APIs 【V — XMLHelp::SaveConfig @0x4BD3B0】:
`SX/SY/SW/SH` (short @0x38..0x46), `TX/TY/TW/TH`, `SC` (attributi float fKX/fKY/fXLC/fYLC),
`TouchRingType` (0x7E/0x80/0x7C/0x82), tags `PenMoveInTablet`, `DeviceCurPenID`,
helper `SetTextValue`.
`MainWindow::GetTabletXmlDataToJson` @0x4706C0 / `GetAPPConfigDataToJson` @0x4519C0 —
**convertono l'intero XML device in JSON per il bus di sync/telemetria** 【I uso endpoint】.

### 7.1 Schema XML config.xml (riassunto operativo)
```
<PenTableLists version="1.5.0">
 └ <MODEL>                     53 modelli (BPH1001, Deco01, …)
    ├ <DeviceInfo>  DeviceRingNum / DeviceKeyNum / DevicePenType / DeviceTrackPad / DeviceControl
    │               [DevicePenNum / PenMoveInTablet / DeviceCurPenID / DeviceKeyDirection / SupportGroupKeys]
    ├ <Common>
    │   ├ <ScreenMapping id="0"> SX SY SW SH
    │   ├ <TabletPannel Orientation="0"> TX TY TW TH
    │   ├ DisableInfo / DisableQuickKey / DisablePressure / DisableSlope
    │   └ <PictureFiles> (coordinate disegno modello in UI: T_0..T_3 con X Y W H)
    ├ <CommonAPP>
    │   ├ <Pen>      PenBtn1(id,Actid), P0/P1X/P1Y/P2, TablePC, RelativeCoords, Speed="5"
    │   ├ <K n>      id / Motid / Actid (K1..K20)
    │   └ <R>        W1..W4 (wheel-modes)
    └ <Apps> <app id path> … per-app override (stesso shape di CommonAPP)
```
Dati aggregati letti a script:
- **DeviceInfo fields totale** = 53 modelli 【V; tabella §8】
- tag counts: `<P**>` 983, `<K n>` 823, `<PenBtn>` 583, `<TablePC>` 244, `<RelativeCoords>` 244, …
- `Actid` usati (sort unique): 1..27, 101..105, 110..111, 201..210 ecc.
- `Motid` 1..20; `Speed="5"` fisso

---

## 8. Database device 53 modelli 【V da config.xml parse】

| Modello | RingNum | KeyNum | PenType | TrackPad | Control | extra |
|--|--:|--:|--:|--:|--:|--|
| BPH1001 | 1 | 6 | 4 | 0 | 0 | |
| EX07W | 0 | 6 | 0 | 0 | 0 | |
| G430 | 0 | 0 | 0 | | | |
| G640 | 0 | 0 | 0 | | | |
| G640S | 0 | 6 | 0 | 0 | 0 | |
| StarG960 | 0 | 4 | 3 | 0 | 0 | |
| Star06 | 1 | 6 | 0 | 0 | 0 | |
| Star03 | 0 | 8 | 0 | 0 | 0 | |
| BPU0908_PH2 | 0 | 4 | 4 | 0 | 0 | |
| Deco01 / Deco01V2 | 0 | 8 | 0 | 0 | 0 / **1** | |
| Deco03 | 1 | 6 | 0 | 0 | 0 | |
| DecoProSW/S/MW | 1 | 8 | 0 | 1 | 0 / 1 / 0 | |
| DecoMini4 | 0 | 6 | 0 | 0 | 0 | |
| DecoMini7 | 0 | 8 | 0 | 0 | 0 | |
| BPU0610W-A | 0 | 10 | 2 | 0 | 0 | KeyDirection=1 |
| BPG0611W-A | 0 | 10 | 0 | 0 | 0 | KeyDirection=1 |
| BPU0410L-B | 0 | 8 | 2 | 0 | 0 | KeyDirection=1 |
| BPU1010W-B | 0 | 12 | 2 | 0 | 0 | KeyDirection=1 |
| BPG0611W-B | 0 | 12 | 0 | 0 | 0 | KeyDirection=1 |
| LPU1505F | 2 | 8 | 1 | 0 | 0 | |
| LPU1506U | 0 | 0 | 4 | 0 | 0 | |
| P16Pro8K | 0 | 8 | 0 | | | |
| LPU2210F-A | 0 | 0 | 1 | 0 | 0 | |
| P24RPro20K2W | 2 | 20 | 0 | 0 | 0 | |
| P1315Pro8K1W | 1 | 8 | 0 | 0 | 0 | |
| P22Pro | 0 | 0 | 0 | 0 | 0 | |
| P126K1W | 1 | 6 | 4 | 0 | 0 | |
| P22EPro16K | 0 | 16 | 0 | 0 | 0 | |
| P156_6K | 0 | 6 | 0 | 0 | 0 | |
| ShortcutRemote | 1 | 10 | 0 | 0 | 0 | |
| ACK05 | 1 | 10 | 0 | 0 | 0 | **SupportGroupKeys=1** |
| LPU2410F-A | 0 | 0 | 0 | 0 | 0 | |
| VB960 | 0 | 8 | 4 | 0 | 0 | |
| BPG0410L-C / BPG0610L-C / BPG1010L-C | 0 | 0 | 0 | 0 | 0 | |
| LPU1505G | 2 | 8 | 5 | 0 | 0 | PenNum=8, MoveInTablet |
| LPU1215 | 0 | 8 | 5 | 0 | 0 | PenNum=8 |
| LPU1211 / LPU1611 | 0 | 0 | 1 | 0 | 0 | |
| Dec1013 | 0 | 8 | 5 | 0 | 0 | PenNum=8 |
| LPH1315F | 0 | 9 | 5 | 0 | 0 | PenNum=8 |
| LPH1015F | 0 | 6 | 5 | 0 | 0 | PenNum=8 |
| LPH1515F | 0 | 10 | 5 | 0 | 0 | PenNum=8 |
| BPU1014L-A | 0 | 16 | 0 | | | |
| DecoMW | 0 | 8 | 5 | 0 | 0 | PenNum=8 |
| LPH1416F-A | 0 | 0 | 6 | 0 | 0 | PenNum=8 |
| BPH1502B | 0 | 0 | 5 | 0 | 1 | PenNum=8 |
| LDN2215F-A | 0 | 0 | 5 | 0 | 0 | PenNum=8 |

### 8.1 Mappatura PID → nome (name_config.ini, completa) 【V】
```
000B Artist 13.3  · 0071 Star 05 · 0074 Star 04 · 0075 Star G540&G430 · 0078 Star 06 · 0081 Star 03
0094/0914 Star G640 · 0096 Deco 03 · 0201 AC19 · 0802 VB960 · 0803 Deco 02 · 080A Artist 12
0902 Deco 01 · 0903 Deco 02 · 0904 Deco Pro Medium · 0905 Deco 01 V2 · 0906 G640S · 0907 STAR 03 V2
0908 Deco 03 · 0909 Deco Pro Small · 090A/900A Artist 16Pro · 090B/900B/091E Artist 22E Pro
090D Artist 15.6 Pro · 0913 G430S · 0915 Star G540 · 0917 G960S · 0918 G960S Plus
091A Artist 15.6 · 091B Artist 22R Pro · 090C/091D/900C Artist 22 Pro · 091F Artist 12 Pro
0920 Star G960 · 0921 Deco S&SW · 0922 Creator CT430 · 0923 Deco L&LW · 0926 G640S
0928 Deco mini7(W) · 0929 Deco mini4 · 092A Artist 13.3 V2 · 092B Artist 13.3 Pro
092C Innovator 16 · 092D Artist 24 Pro · 092E Artist Pro 16TP 4K · 092F Artist 22 (2nd)
0930 Deco Fun XS · 0931 Deco Fun S · 0932 Deco Fun L · 0933 Deco Pro SW · 0934 Deco Pro MW
093A Artist 24           sezione [Dongle]: adatta sottoinsieme
```

---

## 9. UI — struttura `Ui_MainWindow` (204 widget-name registrati) 【V da qstring literals + setupUi】

Layout semantico ricostruito 【V nomi, I raggruppamento】:
```
MainWindow
 ├ main_frame / centralWidget
 │   ├ labelhead_bg / label_wndbg / logo / version / copyright / copyright_2 / statusGroup
 │   ├ groupBox_head / widget_group_head            (header: device pic + battery + settings)
 │   ├ listWidget_left                              (nav dei tabs)
 │   ├ tabWidget
 │   │   ├ tab_device → widget_devicemapping, groupBox_left, tabWidget_screen
 │   │   │      ├ tab_screen: horizontalWidget_4, spinBox_sx/sy/sw/sh, comboBox_screen,
 │   │   │      │    pushButton_screencalibrate, ScreenBtngroupBox, widget_device,
 │   │   │      │    pushButton_fullscreen, pushButton_setscreenarea, pushButton_showscreenid
 │   │   │      └ tab_tablet: widget_tablet_light, LightCheckBox, groupBox_tarea,
 │   │   │           label_rotationtitle, radioButton_r0/r90/r180/r270, pushButton_ScreenSetting,
 │   │   │           TabletBtnGroup, pushButton_fullworkarea, pushButton_screenscal,
 │   │   │           pushButton_selectWorkArea, TabletgroupBox2, label_tx/ty/tw/th,
 │   │   │           spinBox_tx/ty/tw/th, DeConnectLabel/DisconnectTipsLabel/ConnectPenTipsLabel
 │   │   ├ tab_pen → groupBox_pen, horizontalSlider_mspeed, label_mslow, checkBox_mouseMode,
 │   │   │         widget_pressadjust, label_sensitivity, label_hardness,
 │   │   │         horizontalSlider_pressLv, label_curpress, pushButton_ok/cancel,
 │   │   │         groupBox_press, label_press, label_pressvalue_max, label_hard/soft,
 │   │   │         checkBox_windowsInk, pushButton_TestPressure, label_mquick
 │   │   │         groupBox_pen2, comboBox_pSet, comboBox_pbtn0/1/2/3, label_Pen0/1/2/3Show,
 │   │   │         label_penbg, pushButton_pen, DisablePenSettingWidget
 │   │   ├ tab_key → label_keyimage, pushButton_hotkey, groupBox, widget_group_hotkey, scrollArea,
 │   │   │          scrollAreaWidgetContents_2, HotkeyPushButton, label_hotkey_set
 │   │   ├ tab_wheel → label_ringimage, Ring_tabWidget, tab/tab_2, Ring_White_widget(2),
 │   │   │            widget_WheelandTouch, radioButton_m_wheel / radioButton_m_touch,
 │   │   │            RingWheelRadiowidget, comboBox_wheel_1..4/5..8, Wheel_radio_1..4/5..8,
 │   │   │            label_wheelShow_1..4/5..8, comboBox_touch_1..4, Touch_radio_1..4,
 │   │   │            label_TouchShow_1..4, ring_label/ring_label_2, groupBox_brightness,
 │   │   │            label_touch_light, bright0/bright50/bright100, BrightnessCheckBox,
 │   │   │            pushButton_ring, ACK05WheelGroupBox, ImportACK05Button,
 │   │   │            DefaultACK05Button, SaveACK05Button, ACK05GroupBox, scrollArea_2/_3,
 │   │   │            HotkeyPushButton_ACK05, HKWidget, HKCheckButton..4, KeypadWheelLabel,
 │   │   │            KeypadImageLabel, RotateLabel, comboBox_rotate, tab_smallKeypad,
 │   │   │            pushButton_add, pushButton_delete, widget_app
 │   │   └ tab_config → pushButton_import, pushButton_export, checkBox_pressure1,
 │   │                checkBox_hotkey, checkBox_tipMsg
 │   ├ comboBox_pbtn3 / label_Pen3Show / DisablePenSettingWidget
 │   ├ minBtn / closeBtn / settingBtn / YiCiYuanBtn
 └─
```
Slot `on_*` definiti相比 widget 【V — names da simboli】:
`on_checkBox_windowsInk_clicked`, `on_checkBox_mouseMode_clicked`,
`on_horizontalSlider_mspeed_valueChanged`, `on_horizontalSlider_tablet_light_valueChanged`,
`on_BrightnessCheckBox_stateChanged`, `on_spinBox_sx/sy/sw/sh_valueChanged`,
`on_spinBox_tx/ty/tw/th_valueChanged`, `on_comboBox_screen_currentIndexChanged`,
`on_comboBox_rotate_currentIndexChanged`, `on_pushButton_fullscreen_clicked`,
`on_pushButton_fullworkarea_clicked`, `on_pushButton_screenscal_clicked`,
`on_pushButton_setscreenarea_clicked`, `on_pushButton_showscreenid_clicked`,
`on_pushButton_screencalibrate_clicked`, `on_pushButton_selectWorkArea_clicked`,
`on_pushButton_TestPressure_clicked`, `on_pushButton_ok/_cancel_clicked`,
`on_pushButton_import/export` (import/export config), `on_pushButton_add/delete`, …

### 9.1 Inventario dialog (da Ui_*::setupUi in qstring dump) 【V】
```
Ui_MainWindow 204   Ui_HotKeyWin 52    Ui_DisplaySetting 36   Ui_DiagnosisFunction 36
Ui_PenRingSetting 28  Ui_SettingForm 24  Ui_RollerKey 21        Ui_ScreenSettingWidget 19
Ui_WorkAreaSettingWidget 17  Ui_KeyPreviewWindow 17  Ui_DriverDownLoad 16
Ui_DialogEnumProcess 12  Ui_KeySetting 10  Ui_ImportSettingDialog 9
Ui_SelectAreaSettingDialog 8  Ui_DrawLineTestDialog 7  Ui_ShowScreenID 5
Ui_SelectPenMenuWidget 5  Ui_InputKeyForm 5  Ui_HotkeySettingDialog 5
Ui_BatteryWidget 3  Ui_SelectScreenArea 2  Ui_ScreenCalibrate 2  Ui_JingXiMode 2
```

### 9.2 Interfaccia `HotKeyWin` / `RollerKey` / `KeySetting` 【V — HotKeyWin::InitLanguageUI deprecata sezione offsets】
- Gruppi "HotKeySetting.Group1..4" + Actions.* radio/buttons 【v. elenco in vecchio §14 slot table — validi】.
- `hotkeywin_widget_bg`, fadioBtnFun_4..10, radioButton_1/3/4, textEdit, hotkeywin_delete_button,
  hotkeywin_clear_button, lineEdit_2, SelectProBtn, HotKeywin check button 【V da Ui_HotKeyWin 52 nomi】

### 9.3 RCC — 540 file 【estratti】
`/tmp/opencode/xppen-analysis/rcc-extracted/{image/image/tablet (376), image/image/Res (163), image/qss}`.
Per ogni modello existen varianti `<MODEL>.png`, `<MODEL>_K0..K3.png` (orientazioni P0-P3),
`_<R>/_R2.png` (rotazione ruota), `@2x` per retina. 53 modelli ≈ packimgs di 13 MB totali.

`stylesheet.qss` (2173 righe) basato su:
- oggetti `label_wndbg/label_bg/label_title/listWidget_left/pushButton_device|pen|hotkey|ring`,
  `pushButton_cancel/ok save_button_*`, btn_hover/btn_click, ecc.
- palette: bg #333333, list #CCCCCC; radios & box-line UI not presentazione (qualiti testTitolo
  caduto a /image/Res/*.png).

---

## 10. language.ini — spazio traduzioni 【V】

22 sezioni lingua: ChineseSimplified, ChineseTraditional, English, French, German, Italian,
Japanese, Korean, Polish, Portuguese, Russian, Spanish, Thai, Vietnamese, Turkish 【± 6 chiavi per sezione `B`/`E` inserite】.
Key-space per sezione (English group) 【V — counts】:
```
Common 15   Tray 2    Device 23       Pen 21       ShortcutKey 2   HotKey 1
TR 12  Actions 49  RollSetting 8   HotKeySetting 8  Setting 12  SelectAppSetting 2
DiagnosisFunction 17  DisplaySetting 11  Error 6   Information 2  DriverDownLoad 4
Import 9   Update 7   Area 2   WorkareaSetWidget 4   ScreenSetWidget 2
```

### 10.1 Tabella azioni builtin 【V da Actions.* inglese】
```
1 B        2 E          3 Alt        4 Space      5 Ctrl+S     6 Ctrl+Z     7 Ctrl+Alt+Z
8 Ctrl+Shift+Z  9 V     10 L         11 Ctrl+O    12 Ctrl+N    13 Ctrl+Shift+N  14 Ctrl+E
15 F       16 D         17 X         18 Ctrl+Delete  19 Ctrl+C  20 Ctrl+V   21 Ctrl++
22 Ctrl+-  23 Tab       24 F5        25 ]         26 [         27 Eraser
101 Show driver panel   102 Switch monitor   103 Pen/Eraser   104 Precision mode
105 Switch Ring1 function   106 Switch Ring2 function   107 Switch touch mode
109 [B]/[E]             110 Key group switching   111 Set preview
112 The current key group is disabled
201 Shift   202 Left Alt   203 Right Alt   204 Ctrl   205 Space
206 Left click   207 Right click   208 Middle click   209 Left double-click
210 Scroll up     211 Scroll down
```
Questi ID compaiono nei campi XML `Actid`/`Motid`; le stringhe "custom function" usano il
formato `mode|name|data` di §5.5, dove il mode 3 richiama un'azione della tabella builtin
con l'ID indicato 【V struttura; I semantica esatta del campo name lato UI】.

---

## 11. Telemetria / update server 【V — decompilazioni complete】

| Endpoint 【V strings + code】 | Uso |
|--|--|
| `https://driverinfo.xp-pen.com.cn/api/ping` | UpdateServer::CheckUpdateServer (GET; `on_PingFinished`) |
| `https://data.tr.xp-pen.com.cn:2005/receive/data` | JsonServerAction::SendMessageToServerAction — telemetry receiver |
| `https://www.xp-pen.com/feedback?` | link feedback (browse in tab) |

### 11.1 `JsonServerAction` — ciclo telemetria 【V】
- Ctor @0x44AAF0: legge **`<appdir>/dataconfig.ini`** come QSettings(INI) group `config`;
  se contiene un URL override lo usa al posto di quello di default (`this+0x48` = QUrl).
  Directory dati: `QStandardPaths::writableLocation(GenericDataLocation) +
  "/AppData/Roaming/Pentablet V3/data/"` (path letterale in stile Windows, montato così anche su Linux);
  file destinazione log locale `this+0x38` = stesso dir + **`TabletData.txt`**; se `mymac.ini` assente
  → `Get MacPath` + `GetRandonMac`.
- `GetRandonMac` @0x448A20 【V】: charset `0123456789abcdef`, 6 caratteri `qrand()%16` inseriti
  nel template `00:0...` → MAC random persistito in `mymac.ini`.
- `run` @0x44B270: crea QNetworkAccessManager, se flag collect `this[0x52]` attivo →
  `SendMessageToServerAction`, POST header `Content-Type: application/json`, body JSON.
- `onFinishedSlot` @0x449CA0: controlla risposta e attributi (0=status, 2=redirection), error ⇒ flag retry.
- Formato JSON 【V — composizione in `GetJsonData`@0x447E20 e `CAction::GetJsonDataSon`@0x4CC680】:
  `{"mac":"<mac>","time":"<time>","<key>":{...}}` (literals `"","` `":"` `":{` `}` in .rodata
  a 0x512026/0x512024/0x511633/0x51AD2F). I singoli eventi hotkey generano entries
  (`CAction::GetJsonData` per il payload, `GetJsonDataSon` per nesting "PentabletHotkeySetting/HotkeySetting" 【I naming chiave】).

### 11.2 `UpdateServer` — ciclo update 【V】
- `CheckUpdateServer(mode)` @0x4AD840: GET ping; `this+0x14` = mode (2 = auto-download consentito).
- `on_PingFinished` @0x4AE060: parse `QJsonDocument` → due chiavi: `toInt()==1` = "update
  disponibile", seconda chiave = versione/target; con mode==2 → `GetDriverURLString(version)` →
  GET su quel URL (`on_GetDriverFinished`), salvataggio + `GetMD5Sign` (doppio
  `QCryptographicHash::Md5` con `toHex`) per verifica del pacchetto scaricato; dialog
  `DriverDownLoad` (QNetworkDownload) chiude il flusso; gating con `ChangeConfig(bool)`.
- Controlli opt-out 【V】 — `JsonServer::setCollectDataToServerOption`, `MainWindow::InitDataCollectAction`,
  `onChangeCollectDataOption`, `CollectDataChecked`, `Setting_Feedback`: l'utente può disattivare; default 【?】.

---

## 12. IPC: instanza singola & messaggi interni 【V】
- **QtSingleApplication** (`QtSolutions` class, compilata nel binario) con chiave app
  **"PentableV3"**: lockfile `/tmp/qtsingleapp-Pentab-9c9b-lockfile` 【V install.sh + strings】.
  La seconda invocazione invia `sendMessage("Please show PentableV3")` (QLocalSocket) e termina;
  il processo esistente riceve `messageReceived` → `MainWindow::OnShowDriver()`.
- `CReceiveTabletData::PostActionMsg(int,int,int)/ReceivActionMsg(int,int,int)/ReceiveTabletData(TabletDataEX)/SentMultiData`
  【V — dispatcher tra thread e MainWindow; `SentMultiData` passa i MultiData al main loop】.
- `MainWindow::DeleteMapItem(CTablet*)` @0x478770 e `GetSameTablet(int)` @0x478FC0 gestiscono
  rimozione/duplicato istanze per PID 【V symbols】.
- Non esistono altri socket/named-pipe server nel binario.
- **MyLrc::EmitInfo(CGlobalData::Show_Info, testo)** — canale OSD on-screen label utilizzato
  da ChangeBEKey/PressBEKey/DoRingAction per mostrare l'ultima azione 【V chiamate】.

---

## 13. Modello di mappatura (mapping) e calibrazione 【V】
- Per Tab → `ScreenMapping` + `TabletPannel` (attributi SX/SY/SW/SH su schermi virtuali).
  Pipeline calibrazione in `MainWindow`: `SortQRectRect` @0x44ECA0, `EnumDisplay(bool)` @0x44EF80,
  `GetScreenCalibrate`, `SetTabletArea(stTablet_XMLData*)` @0x45B490, `onSetAreaOk(short,short,short,short)`
  @0x45B610, `SetScreenRect` (pick-source: selezione angoli con click del mouse, Esc per annullare
  【da language.ini ScreenHelpInfo1..3】), `checkBox_windowsInk` 【I: scaling pressione】,
  `DidPressurePointsConvertToIntValue`.
- `on_pushButton_selectWorkArea_clicked` / `WorkAreaSettingWidget` definiscono l'area attiva
  della tavoletta (TW/TH) mappata sul screen rect scelto.
- Rotation: `radioButton_r0/90/180/270` + `comboBox_rotate` → valore Orientation `xml+0x48`
  (consumato da `ParsePenDataLinux` §5.2); icone rotante variants `*_R` in resource.rcc.
- "Precision mode" = `OpenFineMode` (azione 104).
- Scroll slider `m_speed` = velocità relativa ring/wheel; `horizontalSlider_pressLv` per il
  livello di pressione (range device `0..0x13D8`, default 5080 = valore da capability string).

---

## 14. Flusso end-to-end 【V ordinato】
```
[USB HID report 64B, poll 100 ms]
        │ (DeviceThread::run 0x4C56C0)
        ▼
DeviceThread::MySentTabletPackets(quint8*, int)   [DirectConnection]
        ▼
CTablet::ReceiveTabletPackets ──(report id)──┬─ dati penna → CParseTabletData::ParseData
        │                                     ├─ ACK/echo B4/B0 → wakeAll cond
        │                                     ├─ 06 'd' → capability update
        │                                     ├─ B6 → pipeline save/read file config
        │                                     ├─ F2 → PostActionMsg(0x58, ...) evento
        │                                     └─ F8 → handshake (PIDs 0x933/0x934)
        ▼
CReceiveTabletData::ReceiveTabletData(TabletDataEX)
        ├── pen    → CAction::ActionTabletData → ParsePenData(Linux) → DoAction
        │                                              ├─ UpdateDigit / UpdateRelMouse (uinput)
        │                                              ├─ DoPenBtnAction → ActionLibrary
        │                                              ├─ DoKeyAction → ActionLibrary/ParseCustomFunction
        │                                              └─ DoRingAction → RingActionLibrary
        ├── keys   → ParseKeyData / ChangeK05DataToKeyState → DoKeyAction
        └── ring   → ParseRingData (lnum/rnum da +0x80/+0x82) → DoRingAction
        ▼
ActionLibrary(id, pressed, extra):
   [sequenze di tasti]  via KeyEvent/KeyEvents
   101..112/Sleep → ShowDriver / ChangeScreenMapping / ChangeBEKey / OpenFineMode /
                    ChangeTrackPadMode / PressBEKey / KeypadPreviewMode / ChangeKeyFunctions
   201..211 → MouserActionLibrary → MyInput::DoMouseFunc(1..6)
   custom  → ParseCustomFunction(mode|name|data|"k+v") oppure run_shell(popen)
   feedback → PostActionMsg(..., 99, "DoKeyAction"+v) per HUD UI
        ▼
MyInput::UpdateDigit / UpdateRelMouse / UpdateWheelRel / KeyEvent — write(input_event, 24B)
        ▼
/dev/input/eventX → libinput / compositor
```
Ogni evento utente invocato aggiorna `CGlobalData::LastTabletActionTime` (QTime::currentTime) e,
se l'azione ha label, `MyLrc::EmitInfo` mostra l'OSD della label corrente.

---

## 15. Compatibilità Ubuntu/GNOME & roadmap del port 【decisionale】

- Il driver oggi richiede **X11**: lineup Qt5 con plugin `qxcb` 【V — pacchetto cosa solo xcb】;
  la UI funziona in XWayland, mentre l'iniezione uinput è indipendente dal display server
  (funziona anche nativamente su Wayland); solo il mapping schermo→tavoletta della UI
  si appoggia a API X11 【V uso Qt5Xcb/X11 paths nel binario】.
- Architettura proposta per il port GNOME (in attesa di approvazione):
  1. **demone systemd --user**: hotplug libusb/hidapi + thread lettore + parse + uinput;
     config in `~/.config/<nome>/` (JSON/GKeyFile), interfaccia D-Bus; nessun processo root
     (udev logind ACL su VID 0x28BD e `/dev/uinput` invece di MODE=0666).
  2. **UI GTK4/Adwaita**: pagina device, pagina penna (curva pressione), editor assegnazioni
     tasti, ring + touch, mapping/area, profili per-app, import/export. UI "mini"/tray opzionale.
  3. **Dispositivi uinput da replicare**: come §6 — tre device (`XP-Pen Pen` abs con tilt/wheel,
     `XP-Pen Mouse` rel, `XP-Pen Eraser` abs) con i medesimi absinfo/keibits per compatibilità
     di risposte con libinput/GNOME 【V parametri esatti】.
  4. Per-app foreground su Wayland: serve `org.freedesktop.portal.RemoteDesktop` o API
     compositor-specific; su X11 `libwnck`.
  5. Telemetria: rimuovere o opt-in esplicito.
  6. Mapping schermi: interrogare Mutter/GNOME (Mutter DisplayConfig D-Bus).

---

## 16. Limiti / open questions 【?】
1. Nessun hardware collegato: **layout esatti dei pacchetti di risposta a runtime** (report 0x06
   subtype 2/3, 0xF2 payload, 0xF8 second stage) restano dedotti staticamente e debbano essere
   confermati con usbmon/strace su device reale. Generazione pacchetti penna
   (`C0/0x20-family`, `0x06`, `0xF6 probe`) è completa 【V statica】.
2. `struct input_event`: codice richiede 24 byte su 64-bit binario; su runtime reale conferma con strace.
3. Semantica precisa dell'azione 88 (emessa dal report `0xF2`, `PostActionMsg(0x58)`) 【?】; identità
   del payload per `KeypadPreviewMode` (stringa) 【?】.
4. Comportamento runtime con GTK compositor/Wayland: la UI è Qt/X11; niente piattaforma Wayland
   plugin nativo 【V pacchetto: solo xcb】.
5. Default telemetria (opt-in/opt-out di fabbrica) 【?】 — desumibile solo runtime o da server.
6. `ChangeK05DataToKeyState` mapping tasti K0..K3 elenco esatto se L/R label differiscono per
   stampa specifica remota 【parziale】.
7. Nota funzionamento integro runtime: `ReceiveUsbMsg` attendo usleep(600ms) prima dell'enum
   — conferma su device reale; `DriveControl` GUI-WinMain interactions non sono approfondite perché
   non presenti nel binario Linux (funzione solo Windows-side).
8. Il file `/tmp/opencode/xppen-analysis/` resta come dump di studio: `nm-defined.txt`, `strings.txt`,
   `qstring-literals.txt`, `rcc-inventory.json`, `rcc-extracted/`.

---

## 17. Evidenze (REA — provider Ghidra 12.1.4, image base 0x400000)
Target: ELF `PenTablet` `sha256 df81b8dde7dcdeb1f992bec210bec3f287fcbb2117fd79e1c34c72aed340f3c9` (1.680.504 B).
Principali procedure decompilate con evidenza 【V】:

- **USB/HID**: `SentCmd` 0x4371E0, `SentSaveFileCmd` 0x436E30, `SentReadFileCmd` 0x436EE0,
  `InitTablet` 0x4380C0, `ReopenTablet` 0x437E40, `SendDeviceChangeMsg` 0x507000,
  `CTablet::ReceiveTabletPackets` 0x4C75D0, `DeviceThread::run` 0x4C56C0, `LinuxRelease` 0x4C5D20,
  `MySentTabletPackets` 0x50A710, `UsbListenThread::*`.
- **Parsing**: `CParseTabletData::ParseData` 0x4C63C0, `ChangeToKeyBoard` 0x4C5E30,
  `ChangeDataToKeyState` 0x4C5F70, `ChangeK05DataToKeyState` 0x4C6150, `ChangeToPenBtnState` 0x4C6340.
- **Action**: tutti gli indirizzi in §5 (0x4CA200..0x4D5FC0) — decompilazione integrale.
- **uinput**: `MyInput::Init/OpenAbsUinput/OpenRelUinput/OpenAbsEraserUinput/UpdateDigit/
  UpdateRelMouse/UpdateWheelRel/KeyEvent/DoMouseFunc/ActionLibrary` (0x42CEA0..0x42F310).
- **Processi**: `main` 0x4273E0, `_start` 0x42B955, `MainWindow::Init` 0x486770, `ReceiveUsbMsg` 0x486840,
  `InitSystemTray` 0x44EA90, `InitKeyUI` 0x44FB00, `InitWheelTouchUI` 0x450350, `EnumDeviceLinux` 0x485940.
- **Config**: XMLHelp `LoadConfig` 0x4B4790, `LoadConfigFromDevice` 0x4BC310, `SaveConfigToDevice` 0x4B4530,
  `ReadDeviceConfigFileToAppConfig` 0x4B28B0.
- **Telemetria**: `JsonServerAction` (ctor 0x44AAF0, run 0x44B270, GetRandonMac 0x448A20,
  SendMessageToServerAction 0x448BA0, onFinishedSlot 0x449CA0, SetCollectOption 0x448A10);
  `UpdateServer` (CheckUpdateServer 0x4AD840, on_PingFinished 0x4AE060, GetMD5Sign 0x4AD9F0).
- **IPC interna**: `CReceiveTabletData::ReceiveTabletDataEX`/`SentMultiData`/`PostActionMsg`/
  `ReceivActionMsg` 0x4F8DE0..; `QtSingleApplication` 0x503310...

Artifacts statici di supporto in `/tmp/opencode/xppen-analysis/`: `nm-defined.txt`, `readelf.txt`,
`strings.txt`, `qstring-literals.txt` (569 literal), `rcc-inventory.json`, `rcc-extracted/`,
`extract_rcc.py`, `decode_qstring_literals.py`.

*Dinamic run evidence: NESSUNA (hardware mai collegato in questa sessione).*
