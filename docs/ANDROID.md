# Vanillish

Vanillish is iNilla on Android: the same client, the same touch controls, controller mapping,
launcher and remote console ([INILLA.md](INILLA.md)), with what an Android phone needs added
around them, and the **Samsung Galaxy Z Fold8 Ultra** first among the phones: its cover screen,
its inner screen and the inner screen half folded on a table each get a layout of their own.
Nothing inside the game is rewritten, and iOS behaves as it did.

It needs Android 12 (API 31) or later, an arm64 phone and Vulkan 1.1. The APK is sideloaded
([ANDROID-BUILD.md](ANDROID-BUILD.md)); the game data comes from your own 1.12.1 client, most
simply through a server of your own and its address ([Quick connect](#quick-connect),
[ANDROID-DATA.md](ANDROID-DATA.md)).

## Quick connect

Vanillish's first screen is **Connect**: one field, **Server address**, and **Connect**. Type the
address of the computer that runs your server — [`server/`](../server/README.md) sets one up in
one command on a Mac with Apple silicon (or Linux, or Windows with WSL) and prints that address —
and the app sets up the rest: the server, the game files and your account. iOS keeps iNilla's
flow; a desktop rehearses this one with `INILLA_QUICK_CONNECT=1` (and `INILLA_METERED=1` or `0`
for the network's answer).

### What to type

| typed | what the app asks |
|---|---|
| `192.168.1.20`, `my-mac.local` | `http://192.168.1.20:8099/server.json`: the server's data host, on its port |
| `192.168.1.20:3724` (the realmlist habit) | the same: 3724 is the realm's port, so 8099 |
| `host:<another port>` | a data host on that port: `http://host:<port>/server.json` |
| `http://…` or `https://…` | `server.json` under that URL (a data host behind a reverse proxy) |

An IPv6 address works bare or in brackets (`[fd00::2]:8099`).

### What happens

1. **The server.** The app reads `server.json` (4 s to connect, 10 s to answer). A Vanillish
   server describes itself there, and becomes a server in the launcher, made the current one: its
   name, its logon (on the host typed, unless it names another), its core, its realm by name when
   it has exactly one, and where its game files are. When every realm runs on that same machine but
   hands out another address (the Mac's old IP, a container's), the world is reached at the address
   typed instead, so a Mac whose address changed works again once the new one is typed. A line says
   what was found (`✔ Vanillish · vmangos · realm Vanillish (PvE) · LAN`), and the server's own
   message, if it has one, shows under it. Connecting to the same server again updates it in place:
   its accounts, saved passwords and language stay, and no second copy appears.
2. **The game files** (the left column). Already all here: "✔ The game files are here — …
   archives, … GB." Otherwise, in turn: the language, when the server has several; the access
   code, when the server's owner set one ([below](#the-access-code)); the list of files; then the
   download, **by itself on an unmetered network** (Wi-Fi) with room for it. On a metered one
   (mobile data, or a hotspot marked metered) the screen says "This phone is on a metered
   connection (mobile data or a hotspot): the download counts against your plan." and waits for
   **Download the game (… GB)**; with too little room the button stays off ("Not enough space: … GB
   needed, … GB free."). It is the setup screen's download: resumable (**Pause**, **Resume the
   download**), every archive checked against its SHA-256, the server's addons (pfQuest) installed
   after the archives. A server that shares no game files says "This server does not share the
   game files. Bring your own:" over **Import from folder…**.
3. **The account** (the right column, "Your account on" the server), meanwhile: sign-up needs no
   game files. **Create an account** comes filled in, with a name (`PLAYER` and four digits) and a
   generated password, both editable; **Create account** creates it ([Sign-up](#sign-up)). **I have
   an account** takes a name and a password and **Save**s them: nothing is sent, and a wrong
   password shows at the login. Both start with **Remember the password (sealed on this phone with
   an Android Keystore key)** and **Log in automatically when Vanillish starts** ticked.
4. **Play.** Once the download is done: "Vanillish opens the archives when it starts: restart it to
   play.", and **Restart Vanillish now**. Vanillish starts again by itself, in a fresh process (the
   game opens its archives only at start), and the saved account logs in by itself, on the realm
   picked by name. Until then the account card says "Vanillish logs in as NAME once the game files
   are in." (then "… when it starts again."). When the game files were already open at launch, the
   card has **▶ Play as NAME** instead.

The relaunch carries the whole flow: the server and the account are in `servers.toml` (a server
Connect set up also keeps `info`, where it described itself, and its data's `locale`, the language
picked, so a resumed download stays in it), the password and the access code in the Keystore. A
Connect left before an account was made opens again at the next launch.

### Sign-up

- **The name**: 3 to 16 letters or digits (ASCII), stored in capitals. A server may set other
  lengths in `server.json`, for the name and the password, never more than 16 (vmangos' limit).
- **The password**: 6 to 16 characters — letters, digits and symbols, without spaces, `"`, `'` or
  `\`. Capitals do not matter: 1.12 sends passwords in capitals. The generated one is 12 capitals
  and digits, without 0, O, 1 or I.
- **The server never sees the password.** The phone works out the account's SRP6 salt and verifier
  (benilla's own SRP6, the code that logs in) and sends only them, to the data host's
  `POST /accounts`. The generated password is shown once, in clear on the form (**Hide** hides it);
  once the account is created it is sealed in the Keystore (with **Remember the password** ticked)
  and nothing shows it again: note it down to log in from another device.
- **A refusal says why**: "That account name is taken. If it is yours, choose "I have an
  account"."; "This server does not take sign-ups from here." (sign-up off, or the phone not on a
  private network); "Too many new accounts from here: try again in N minutes." (5 in 10 minutes and
  20 a day per address, 30 an hour in all); "The server is still starting. Try again in a minute.".
  When no answer came, **Create account** again sends the very same request, so the account is
  never made twice.
- A server that takes no sign-ups from the app shows only **I have an account**, with "This server
  does not take sign-ups from the app: ask its owner for an account, then enter it here." No
  account made in the app is a game master.

### The access code

A server whose owner set `INILLA_DATA_TOKEN` says so in `server.json`, which stays open. The files
column then asks for the **Access code**, and **Continue** seals it in the Keystore for that server
and sends it with the download and the sign-up (as `Authorization: Bearer`); the account card asks
for it only when the files column is not already asking. A code the server refuses is forgotten
and asked for again: "The server did not accept that access code." Deleting the server in the
launcher deletes its code too.

### When the server does not describe itself

| what answers | what Connect does |
|---|---|
| an iNilla data host from before `server.json` (its `/` starts with `inilla-datahost`) | its game files alone: a server on the typed host and port 3724 with that data source, under "This server shares its game files, but says nothing about its realm: it is reached on the usual port 3724."; no sign-up, so **I have an account** |
| nothing on 8099, but a logon server on 3724 (a 2 s TCP connect) | a plain 1.12.1 realm: the launcher's server form, filled in with the host, under "A 1.12.1 realm answers at \<host\>, but it does not describe itself: no game files to download, no sign-up. Check the details and save." (the edit form, when the launcher has that server already) |
| nothing | "Nothing answers at \<host\>. Is the server running, and is this phone on the same Wi-Fi as it (not a guest network, not mobile data)? On the server's computer the firewall must let in ports 3724, 8085 and 8099.", the network's own reason under it |
| a `server.json` this version cannot use (another format, another client build) | "Vanillish cannot use this server: …" — for a newer format, "update the app" |

When Connect opens again for a server it set up (after a relaunch, from the launcher), it asks the
server again, once. If only the realm answers: "Only the realm answers at \<host\>: the server's
data host (port 8099) does not, so the game files and sign-up wait until it is running again."

### Around it

- Without the game data, Connect is the first screen, Back on the other screens leads to it, and it
  has no **× Close** until the data is here. A current server without a data source (entered by
  hand without a data URL, or one that shares no game files) starts at the setup screen instead,
  as before.
- With the game data, a launch with no server yet (the files came another way), or with a server
  Connect set up and no account on it yet, opens Connect; otherwise the automatic login or the
  launcher, as before.
- **Enter the details myself** opens the launcher's server form (iNilla's). **Other ways to get the
  game files…** opens the setup screen ([ANDROID-DATA.md](ANDROID-DATA.md)) with this server's data
  URL and access code filled in; its **Connect** button comes back.
- In the launcher, **Connect to a server…** takes the place of **+ Add server**, and a server
  Connect set up has **Create account**, which opens Connect's sign-up for another account (as
  **Use another account** on Connect does).

## The server admin menu

On a server of your own ([`server/`](../server/README.md#the-admin-menu)), **Settings → Server** is
its owner's admin menu: the realm's experience, loot, gold and speed rates, for everyone, changed
while you play and kept when the server restarts, and gold for players. Open Settings with the
**N** button in the top strip of the touch controls, then the **Server** tab (iOS has none). It is
for the launcher's current server, under its name and the address it is asked at.

- **The admin code, once per server.** The tab first says "The admin code is in the server's .env
  (ADMIN_CODE), and at the end of its setup." Type it (dashes and capitals do not matter; a plain
  keyboard, no suggestions) and **Continue**: nothing is sent before. A code that cannot be one is
  refused on the phone ("An admin code is 16 letters and digits, as the setup prints it (or 20 hex
  digits).", "The admin code takes letters and digits only, …", "That is the example's placeholder:
  …"), and one of the setup's length with a 0, 1, I or O in it (a misread letter) is told so:
  "The setup's codes have no 0, 1, I or O: check those letters (an O is a Q or D, an I is an L or
  J)." The code is sealed with the Android Keystore key, as the passwords are
  ([Passwords](#passwords)), one per server, and never sent: each request carries a signature made
  with it over a one-time number from the server. **Forget the code** at the bottom drops it; a
  code the server refuses (as wrong, or as one of too many wrong ones from here) is dropped and
  asked for again; deleting the server in the launcher deletes it too. When the Keystore refuses
  it: "The admin code was not saved: the Android Keystore refused it. Vanillish keeps it until it
  closes."
- **The status line**, with **Refresh**: "The world server is up: 2 players online." (or the reason
  it is not, below). The tab asks when it comes up and again 15 seconds after each answer while it
  shows, and a refresh never moves a slider you moved and have not applied (the others follow the
  server).
- **Experience**: **All experience** sets kills, quests and exploration together; **Separately**
  opens them one by one (Kills, Quests, Exploration).
- **Loot**: **Item drops** sets every quality together; **Separately**: Grey (poor), White (and
  quest items), Green, Blue, Purple, Orange, and Shared loot tables (how often one is rolled).
  **Gold from loot and quest rewards** is a slider of its own. Under them: "From the next kill. Boss
  and dungeon loot that drops one item from a list does not change." (vmangos rolls those lists with
  no rate.)
- **Movement**: **Player speed**, walking, running, swimming and mounted (taxis keep their own).
  A server whose vmangos lacks Vanillish's speed patch has none: "This server's vmangos has no speed
  setting: run its setup again to update it."
- **The sliders** run from ×0.5 to ×10 (speed ×0.75 to ×2), on a log scale, in steps of 0.05 (speed
  0.01); a rate the server already holds outside that range widens it, and is never moved. A group
  slider whose rates differ says "mixed", its handle between them, and moving it sets them all.
  Under a rate the menu changed, the server's own value: "server ×1". While the world server is not
  up or a change is on its way, the sliders are read-only.
- **Apply** and **Undo** show once a slider moved, with "Applying also resets the message of the day
  and the player limit to the server's settings." (the server reloads its settings to apply). Then
  "Applied: live now, and kept when the server restarts." **Tell the players in chat** (on) has the
  server announce the new rates in chat (`Server rates: XP x2, loot x1.5, gold x1, speed x1.2`).
  **Server defaults**, while a rate the menu set is saved on the server (a speed saved before its
  vmangos lost the speed patch counts too), takes two taps ("Tap again to go back to the server's
  settings") and puts every rate back to the server's own `config/mangosd.conf`, such a speed
  dropped too: "Back to the server's own settings."
- **Give gold**: the players online as buttons (name · level), **Everyone online**, and **Other
  name** for anyone else, online or not (2 to 12 letters); at most 50 names at a time ("Pick up to
  50 players, or Everyone online."). An amount of 1g, 10g, 100g or 1000g, or the slider between 1
  and 1000 gold. **Send** takes two taps ("Send 100g to 3 players? Tap again"). The gold
  comes **by mail**, "A gift from the realm": "Sent by mail to Thrall, Jaina: they collect it at any
  mailbox." (the world server answered with its own "mail sent" sentence, in the realm's
  language); "No character named Arthas."; "Maybe sent to …: the server did not say. Check a
  mailbox before sending to them again." (any other answer, or none in time); "Not sent to …: the
  server ran out of time.", with **Send to the N not sent**, which sends the same amount to those
  names alone, in batches of at most 50. With no answer at all (or a proxy's error page in its
  place): "No answer in time: the gold may still arrive. Check a mailbox before sending again.", and
  **Send** for the same players and amount goes again as the very same request, which the server
  answers without paying twice (unless its world server restarted meanwhile, which forgets the
  request: hence the mailbox).
- **Its fields keep their keys.** While a field on Vanillish's screens has the keyboard (the code,
  **Other name**, any other), the keys typed are the field's alone: in the world, typing there moves
  nobody and its Enter opens no chat. A key already held when the field took the keyboard still
  lets go.

The tab asks the server's data host, at the address Connect found it (a server entered by hand: its
host, on port 8099): `server.json` first, which says whether the server has the menu, then, with a
code, a one-time number and the signed request (4 s to connect, 30 s to answer, no redirects).

| the tab says | what it means |
|---|---|
| "This server has no admin menu: update it (run its setup again), and set ADMIN_CODE." | its `server.json` has no `admin`: no usable `ADMIN_CODE`, or a data host older than the menu |
| "The admin menu answers only on the server's own network." | `INILLA_ADMIN=lan`, and this phone is not on a private network (mobile data) |
| "The admin menu is off on this server." | `INILLA_ADMIN=off` |
| "That is not this server's admin code." | the code was refused, and forgotten: type it again |
| "Too many wrong codes from here lately: check the admin code (ADMIN_CODE in the server's .env)." | the code is wrong, after ten wrong codes from this address in ten minutes (behind Docker Desktop, from anyone): forgotten, type it again; the right code still gets in |
| "Too many requests from here lately: wait a moment and try again." | a 429 that is not the data host's own (a proxy in front of it limiting requests): the code is kept |
| "The world server is starting…" | the world server is not up yet, or stopped; the sliders wait |
| "No answer in time: it may still have been done. Refresh before trying again." | the request reached the server and no answer came (none in time, the world server stopped or restarted meanwhile, or for a change, a proxy's error page in its place): Refresh shows what took |
| the server's own sentences: "The data host and the world server disagree on ADMIN_CODE: run `docker compose up -d` on the server.", "The world server's console is busy: try again in a moment.", … | as they say; the error's code shows under them |
| "This server is older than the admin menu: update it (run its setup again)." | an answer that is not the menu's, where the menu should be |
| "The server did not see this request's signature: a proxy in front of it may drop the Authorization header." | a reverse proxy that drops `Authorization` |
| "Nothing answers at \<host\>. …" | as on Connect, the network's own reason under it |
| "No server yet: add one in Servers first." | the launcher has no current server |

## Where it lives

| path | what |
|---|---|
| `crates/vanillish-android` | the app's entry: `libvanillish.so`, whose `android_main` (on GameActivity's native thread) runs `inilla_boot` and `benilla_app::run`; `src/jni_exports.rs` holds every function the Java side calls. |
| `crates/inilla-platform/src/android/` | the OS glue behind iNilla's plain Rust calls: JNI into the Java `Bridge` class (`with_env`, `call_bridge`), the app's folders, the clipboard, the device (screen on, haptics, safe area, memory, frame rate, orientation, game mode, Back), the fold, the Keystore, logcat, the thermal state (`AThermal`). |
| `crates/inilla-platform/src/{gamepad/android,pointer,picker}.rs` | the controller store, the mouse and pen queue, the folder picker: plain Rust the JNI shims feed, tested on any host. |
| `crates/inilla-core/src/{fold,import}.rs` | the screen classes, their layouts, the tabletop split, the interface scale per screen; copying the game data in from a picked folder. Engine-free, unit-tested. |
| `crates/inilla-core/src/connect.rs` | quick connect: the address typed, `server.json`, the server profile it makes, the game files' next step, the sign-up's checks, body and reply. Engine-free (the network and SRP6 are the caller's), unit-tested. |
| `crates/benilla-app/src/inilla/{connect,ui/connect}.rs` | quick connect's jobs (the probe, the manifest-then-download chain, the sign-up with benilla's SRP6, the account saved) and its screen. |
| `crates/inilla-core/src/admin.rs` | the server admin menu's engine-free half: the admin code's rule, the request's HMAC-SHA256 signature, the data host's replies and refusals, the rates and their groups, the sliders' draft and ranges, gold in whole gold, the request bodies. Unit-tested (the HMAC against RFC 4231, and the vectors the data host's tests share). |
| `crates/benilla-app/src/inilla/{admin,ui/admin}.rs` | the admin menu's requests (`server.json`, the nonce, the signed request, on one worker thread; the code read from the Keystore once per server; a gold send's id kept after no answer) and Settings → Server. |
| `crates/benilla-app/src/inilla/android.rs` | the Bevy side: the soft keyboard's text, Back, the native window's size, leaving and coming back to the screen, the frame-rate vote, Game Dashboard's modes, the session service. |
| `crates/benilla-app/src/inilla/screen.rs` | which screen the game is on, its layout, the world above the hinge, the orientation, the edges the back swipe holds off. |
| `crates/benilla-app/src/inilla/host_pointer.rs` | a hardware mouse and a hovering pen, into iNilla's virtual mouse. |
| `crates/benilla-app/src/inilla/ui/words.rs` | every sentence that names the platform or the app: iNilla's on iOS, Vanillish's on Android. |
| `android/` | the Gradle project: the Java classes (`VanillishActivity`, `Bridge`, `GamepadBridge`, `PointerBridge`, `FolderImport`, `FoldTracker`, `GameSessionService`, `Relauncher`, in `io.github.klanthier.vanillish`), the manifest, the resources, `build-rust.sh`, `build-apk.sh`, `tools/`. |
| `.github/workflows/vanillish-*.yml` | the gate on every push, and the APK pipeline. |
| `server/`, `.github/workflows/server-*.yml` | the server quick connect talks to: vmangos and the data host in Docker, the one-command setup, its check and its images ([server/README.md](../server/README.md)). |

Rust asks Java for things through static methods on `Bridge` (it pulls: insets, the fold and Back
presses are cached there for it to read); Java hands Rust events through `static native` methods,
which only pass plain values on into `inilla-platform`. `inilla-platform` stays free of Bevy and
of android-activity; what needs Bevy's `AndroidApp` lives in `crates/benilla-app/src/inilla/`.

## How input reaches the game on Android

Through iNilla's two seams and nothing else — commands by name (`inilla/synth.rs`) and an
ordinary mouse (`inilla/mouse.rs`) — plus ordinary key messages for typing:

| input | path |
|---|---|
| fingers | GameActivity → winit's touch events → iNilla's touch engine, as on iOS. At most eight fingers: a ninth is dropped before winit sees it. |
| the soft keyboard | GameTextInput's text buffer, read every frame by the IME bridge (`inilla::android`) → characters and Backspace as key messages, Enter and Tab as keys pressed for the player. |
| a hardware keyboard | winit's key events, as on the desktop (a hardware Enter is rewritten to the Enter key). |
| Back | the Back key or gesture, as a key (predictive back is off) → winit → `BackPresses` → iNilla's screens first, then the game's Escape. |
| a controller | `GamepadBridge.java` → `jni_exports.rs` → `inilla_platform::gamepad::android` → iNilla's mapper, unchanged. |
| a mouse, a trackpad, a hovering pen | `PointerBridge.java` → `jni_exports.rs` → `inilla_platform::pointer` → `inilla::host_pointer` → the virtual mouse. |

So, as on iOS, nothing Vanillish does is something a keyboard and mouse player could not do.

## The Galaxy Z Fold8 Ultra

### Three screens, three layouts

| where you play | class (Settings → Touch says) | size in landscape | layout file | the layout |
|---|---|---|---|---|
| the cover screen, folded | Phone / cover screen | ≈ 960 × 411 dp | `touch.toml` | iNilla's phone layout, unchanged: the cover screen is about an iPhone Pro Max's size and shape. |
| the inner screen, unfolded | Inner screen | ≈ 954 × 859 dp | `touch-tablet.toml` | for two hands holding the side edges: the stick under the left thumb, jump under the right one with slots 1–6 around it and 7–12 on an outer arc toward the middle (Combat view); target (Tab), interact (Use) and autorun (Run) above the stick; the ☰, view and menu buttons at the top centre. Everything is anchored to the side edges, so it stays under the thumbs on a taller or shorter window. |
| the inner screen half folded, hinge across, on a table | Tabletop (half folded) | the world ≈ 954 × 430 dp above the hinge, the deck ≈ 954 × 418 dp below | `touch-tabletop.toml` | the controls on the deck below the hinge, played from above with the fingers of both hands: the stick on the left, jump and twelve slots on the right, target and interact at the bottom middle, the system buttons along the crease. |

The sizes are at a density of 2.625; the phone may use another. The rule behind the classes
(`inilla_core::fold::classify`): a window whose short side is at least 600 dp and whose long side
is at most 1.6 times it is the inner-screen kind; anything wider or smaller is a phone; half open
with the hinge across, and at least 240 dp on either side of it, is a tabletop. So another phone
gets iNilla's layout, a small tablet the inner screen's, and a split-screen or DeX window
whichever its shape is.

Each screen keeps its own layout, and an edit changes only the current screen's. The right one
comes up by itself a moment (0.35 s) after you fold or unfold: the outgoing layout is saved, every
finger is let go of, and the stock interface is re-anchored. **Settings → Touch** shows the screen
you are on, has **Copy the phone layout here** to start the inner screen or the tabletop from your
phone layout, and **Reset layout** puts back the current screen's own shipped layout. **Reset all
Vanillish settings** (Settings → About) resets every screen's layout.

On the inner screen and the tabletop, the hub's **More** opens the touch radial menu at your thumb,
and the radial is no bigger than a phone's (at most 170 dp from its centre), so every wedge stays
within the thumb's reach. The cover screen keeps iNilla's radial, as does iOS, iPads included.

The interface size follows the screen too (Settings → General → Interface size, Automatic):

| screen | automatic `uiScale` | why |
|---|---|---|
| cover | 1.25 | iNilla's phone rule: a stock action button near fingertip size. |
| inner | ≈ 0.83 | a fingertip button would leave the near-square screen narrower than the 1024-unit main action bar; the scale stops where the bar still fits. |
| tabletop | 0.64 | the phone rule applied to the half above the hinge: about 600 units of interface there, near a phone's 614, so quest and vendor windows fit above the crease. 0.64 is the smallest scale the 1.12 client applies. |

A pinned interface size (Automatic off) applies as set on the cover and inner screens; on a
tabletop it is applied to the half above the hinge as it would be to a screen that tall (never
below 0.64).

### Tabletop (Flex mode)

Half fold the unfolded phone, stand it on a table with the hinge across, and in the world:

- the 3D world and the stock interface — windows, action bars, chat — end at the hinge, and the
  nameplates and chat bubbles stay over their units above it;
- below it the HUD has a black deck to itself, with the crease drawn faintly as a line the fingers
  should not rest on;
- a drag on the deck's empty middle looks around (a look pad); a tap there clicks nothing;
- the deck has no world under it, so nothing there points into one: a mouse, a hovering pen and
  the controller's cursor reach the world above the hinge only (over the deck the pen shows no
  cursor, and a mouse neither hovers, clicks nor starts mouse-look);
- the controller's radial menu opens in the middle of the world above the hinge, and its cursor
  stays above the hinge.

The login and character screens stay whole-screen; the split is in the world only, and only while
the hinge still reads half open. The layout editor still places the tabletop's controls on the
deck, wherever you open it (the character screen too), so they land where the world will have them.
**Settings → Touch → Tabletop: the game above the hinge, the controls below it** turns the split
off; a half-folded inner screen is then the inner screen.

Stock windows taller than the half above the hinge (about 600 units), long dropdowns, and addons
that size themselves with `GetScreenHeight()` can reach into the deck. Open bags do not: their
stack starts a new column where the half above the hinge ends.

### Orientation

The game plays landscape, either way up. Unfolded, **Settings → General → Inner screen: follow the
device into portrait** lets it turn with the phone (off by default); the cover screen always stays
landscape, since the touch controls and iNilla's screens need its width, and so does a tabletop,
since that is what puts the hinge across. In split screen and DeX windows Android ignores the
game's orientation.

### Cut-outs and the safe area

The game draws under the camera cut-out on every edge, full screen, with the system bars hidden (a
swipe from an edge shows them for a moment). What the HUD and the stock interface keep clear of:

- **the camera cut-out.** On the cover screen in landscape the hole sits mid-height on the left or
  right edge — where the Dynamic Island is on an iPhone — and **Settings → General → Keep windows
  clear of the camera cutout** stops the stock windows at it on that side (the 3D world still fills
  the screen). The side comes from the cut-out's own bounding rects, so a curved edge is not taken
  for a camera. A cut-out on the top edge insets the top. Where the inner screen's camera lands in
  landscape is still to be seen on a device;
- **the home swipe strip** along the bottom, which Android keeps whatever the app asks: the bottom
  row of controls stays above it;
- **in a window** (split screen, pop-up view, Samsung DeX): the system bars and the window's title
  bar too.

While the touch controls show, Android's back swipe is held off along the side edges beside the
elements a thumb rests on (a 32 dp band, at most 200 dp of height per edge — Android's limit), so
a thumb landing on the stick does not leave the game. The bottom edge's home gesture cannot be
held off.

## Frame rate: 120 Hz and the presets

The graphics preset (Settings → General → Graphics) asks the panel for its rate with the call
Android recommends for games (`Surface.setFrameRate` on the game's surface, again on every new
surface: each panel of the Fold has its own modes):

| preset | frame rate | the panel is asked for | view distance (`farclip`), ground clutter |
|---|---|---|---|
| Battery saver | 30 fps | 30 Hz | 177, 16 |
| Balanced (the default) | 60 fps | 60 Hz | 330, 32 |
| High | the panel's full rate (120 on the Fold8 Ultra) | its highest | 777, 64 |
| Custom | the panel's full rate | its highest | the stock video options, untouched |

The labels show the rates (`High 120 fps`, read from the panel). The LTPO panel then runs at the
preset's rate instead of idling at 120 against a capped game. When the panel reports the cap's
rate and frames come paced, iNilla's sleeping frame cap stands down (presenting already paces
them); if frames still come faster than reported, it caps again for ten seconds.

High on the inner screen draws twice the pixels of the cover screen at twice Balanced's rate: it is
best on a charger. Render resolution (80 % by default) lowers the 3D world's pixels and leaves the
interface sharp. Samsung's own settings can hold a game below what it asks: Settings → Display →
Motion smoothness → Standard caps the phone at 60 Hz, and Game Booster has frame-rate settings of
its own.

### Game Dashboard

Android's game modes map onto the presets: **Performance** picks High, **Battery** picks Battery
saver. Only when the mode *changes* — the mode last seen is kept in `settings.toml`
(`last_game_mode`), so a launch in the mode the game was left in changes nothing — never over a
Custom preset, and a preset you pick afterwards stands until the mode changes again. The mode is
read on every resume (a developer can set it with `adb shell cmd game mode performance
io.github.klanthier.vanillish`). The manifest declares both modes and lets the system neither
downscale the game nor cap its frame rate: Vanillish has its own render resolution and cap.

### Heat and memory

Settings → Debug → **Show frame rate, memory and heat** puts both on screen: memory is the
system-wide room left before Android's low-memory killer starts on background apps (Android has no
per-app limit like iOS's), and heat is the thermal state — `warm`, `HOT`, `THROTTLING` — from the
system's thermal service. The Fold8 Ultra cools with graphite sheets, without a vapour chamber, so
a long session at High is likely to throttle; Balanced, the default, is the preset for long play.
Nothing steps the preset down by itself yet.

## Modern lighting

Settings → General → **Modern lighting (restart)**, off by default, keeps the game's hand-painted
textures and changes how they are lit. It applies at the next start. With it on:

- **The stock light, plus relief.** The scene's lighting is the stock look's, computed the same
  way: the same brightness, colours and sunlit glare on the ground. On top of it, each texel's
  brightness is read as height (four extra texture reads per pixel), and the light moves around
  the stock value by that relief: the side of a crack, a brick joint or a plank seam that faces
  the sun brightens and the other darkens by as much, so the surface reads as 3D without getting
  darker or lighter overall. The cracks themselves shade a little darker and the tops a little
  lighter. No new art, and no tangent data in the models.
- **A glinting glare.** The ground's sun glare stays where the stock look puts it and sparkles
  along the relief's edges, with a small, sharp hot spot (terrain's own highlight masks, the `_s`
  textures, still decide where it shows).
- **A soft sheen.** A small, warm highlight from the sun on bright texels (painted metal, glazed
  stone, leaves) of outdoor models and buildings. None indoors or in shade.
- **Round torch pools.** The three point lights each surface already picks are lit per pixel, so a
  torch throws a round pool instead of a diamond on the ground.
- **Water.** The sun's glint per pixel, and a grazing view lifts the water toward the sky's colour
  and opacity.

Nothing else changes: the same passes, the colours written in the same form, the interface and the
glow untouched. The cost is shader work: four texture reads and a few dozen operations per lit
pixel (terrain: four per texture layer it blends, mostly one or two). Try it at 60 fps first, and
watch **Show frame rate, memory and heat** (above) at 120.

On a computer the same look is `/console modernLighting 1` (or the row in F10's Settings, which
shows and writes that CVar) and a restart, or, for one session, `VANILLISH_MODERN=1` (which wins
over the setting). `VANILLISH_RELIEF` and `VANILLISH_SHEEN` set
the relief's and the sheen's strength in percent (100 by default, 0 to 400; at 0 each, land and
buildings are lit as in the stock look, but for the round torch pools). The world viewer shows a
before and after of one spot:

```sh
WOW_MAP=1 WOW_WORLDVIEW_AT=-618.5,-4251.7,38.7 WOW_CLOCK=600 \
  WOW_WORLDVIEW_SHOT=/tmp/before.png cargo run -p benilla-worldview
VANILLISH_MODERN=1 WOW_MAP=1 WOW_WORLDVIEW_AT=-618.5,-4251.7,38.7 WOW_CLOCK=600 \
  WOW_WORLDVIEW_SHOT=/tmp/after.png cargo run -p benilla-worldview
```

(the Valley of Trials, at ten in the morning; `cargo test -p benilla-assets --lib modern_light`
checks that every world shader still compiles both ways, and that the relief averages to the stock
light at the tuned strengths).

## The soft keyboard and the chat bar

Vanillish types with the system keyboard (Gboard, Samsung Keyboard), suggestions and autocorrect
included. A chat box raises it when it takes the focus, and so does a field on iNilla's own
screens (Settings → General → **Show the keyboard when a text box is focused**, as on iOS); on the
login and character screens the **⌨ Keyboard** button at the top right raises it.

- The keyboard floats over the game; the game is never resized under it. In landscape it covers
  the stock edit box, so the HUD's **chat bar** at the top shows the line being typed, with
  **Send** and **× Close** (Close is the Escape key, as on iOS).
- The keyboard holds the word you are still typing (underlined in its suggestions) until a space,
  a punctuation mark or a picked suggestion commits it. The chat bar shows that word after the
  line, dimmed and underlined; the game gets it when it is committed, or when the line is sent or
  closed.
- Return on the keyboard, or the chat bar's Send, sends the line. Backspace in an empty box still
  reaches the game.
- Password fields open a plain keyboard with no suggestions, and the keyboard learns nothing from
  them; address fields (the realmlist, the data URL) get the URL layout. The keyboard never takes
  the whole screen in landscape.
- A Bluetooth or DeX keyboard types too, also while the on-screen keyboard is up.
- While a field on Vanillish's own screens has the keyboard, the keys typed are the field's alone:
  in the world, typing in one moves nobody and its Enter opens no chat.

## Back

The Back gesture or key is the Escape key iOS has none of, and it never leaves the game:

1. With the on-screen keyboard up, the first Back closes the keyboard (Android's own rule).
2. On iNilla's screens it undoes the most local thing first: an open dropdown, the password prompt,
   the server or account form, the layout editor (Done), the log viewer (back to Settings), then
   the screen itself (without game data, back to the first screen: Connect, or the setup screen
   when that came first).
3. Anything left is the game's Escape: it closes the open window, or opens the game menu.

Vanillish opts out of predictive back (the manifest's `enableOnBackInvokedCallback="false"`), which
Android 16 turns on by default: on every version the gesture reaches the game as the Back key, as
on Android 12–15. Predictive back would take every Back key first, whatever sent it; without it, a
controller's Back button stays the controller's (the game menu, below) and a mouse's side button
stays a mouse button (below). A controller's B never acts as Back.

## Controllers

Anything Android sees as a gamepad: Xbox, DualSense, DualShock 4, Switch Pro, the Backbone One and
other USB or Bluetooth pads. Pair one in the phone's **Settings → Connections → Bluetooth**, or plug
in a USB-C one; it works with no setup, in iNilla's mapping and with iNilla's settings
([INILLA.md](INILLA.md#controllers)).

- **The cover screen with a phone clip.** A USB-C clip (Backbone One, Razer Kishi, GameSir) on the
  folded phone plays on the cover screen, the Fold as a handheld — if the clip takes the folded
  phone's size. Unfolded, use a Bluetooth pad.
- **The pad's own symbols.** The HUD and the stock buttons' hints show ×○□△ for Sony pads and the
  Backbone One PlayStation Edition, Nintendo letters for Switch pads, Xbox letters otherwise.
- **More than one pad.** The one you last used — a button, or a stick or trigger past halfway —
  is in control, so a clip on the cover screen and a pad on the table both work without
  disconnecting either.
- **Home** goes to the Android home screen. **B** is the game's B, never Back. Start/☰ opens the
  radial menu, Select/View/Create (Back on cheaper pads, which is never Android's Back) the game
  menu, as on iOS.
- Pulling down the notification shade, folding the phone or anything else that takes the focus
  releases every held button.
- Very quick taps — down and up between two frames — still count.
- Rumble is not supported yet.

## DeX, a mouse, a keyboard, a pen

A hardware mouse, a trackpad or a keyboard works in **Samsung DeX** or connected over Bluetooth or
USB, as on the desktop client:

- left and right click, and hovering for tooltips; the pointer shows the game's own cursors (the
  sword, the hand, a held item's icon);
- the wheel zooms the camera and scrolls lists; a trackpad's two-finger swipe does the same;
- the wheel button and the two side buttons are `BUTTON3`, `BUTTON4` (autorun by default) and
  `BUTTON5` for key bindings. The side "back" button is `BUTTON5` alone, never Android's Back, on
  every Android version;
- holding the right button turns the camera; a mouse's pointer is captured and hidden meanwhile and
  comes back where it was. A trackpad is never captured: mouse-look on one follows its pointer and
  stops at the screen's edge.

While a mouse or a hardware keyboard is in use, the touch controls step aside; the next touch
brings them back. **Settings → General → Hide the touch controls while a mouse or keyboard is in
use** turns that off. The on-screen keyboard does not count.

In DeX the game runs in a window: its layout follows the window's shape (a wide window is laid out
as a phone), and the safe area keeps clear of the window's title bar.

The Fold8 Ultra takes no S Pen; on a phone or tablet with a stylus, a pen hovering above the screen
is the mouse without a button — tooltips and highlights under it, and a touch control's tooltip
without pressing it. Touching the screen with the pen is a tap.

## Staying connected

Android freezes an app's process within seconds of it leaving the screen, and the server drops a
character whose client stops answering. **Settings → General → Stay connected in the background**
(on by default) keeps the session: while you are in the world, a foreground service keeps the
process, and the game's connection, alive while you look at another app or lock the phone. A quiet
**"Staying connected to the game server. Tap to return."** notification shows meanwhile (channel
*Game connection*, white V icon); tapping it returns to the game.

- It runs only in the world, and stops when you leave the world, turn the setting off, quit, or
  swipe Vanillish away from Recents — and when the session ends while the game is off the screen
  (the logout below, or a lost connection).
- The first time, Android 13 and later ask for permission to show notifications. Without it the
  service runs all the same; the game then shows only in the system's list of running apps.
- The world does not update while the game is off the screen; the connection stays up, and what the
  server sent waits for your return.
- Not for ever: 30 minutes after your last input — counted on the clock, through a locked phone's
  sleep — the game asks the server to log the character out, the idle logout it would make on the
  screen. If the character is still in the world 5 minutes later (the server refuses a logout in
  combat), the connection is closed. Either way the character leaves the world, and the service
  stops.
- Leaving the screen, Vanillish saves its settings and layouts, lets go of every held key and mutes
  its sound; coming back, the sound returns.
- How a fold behaves depends on Samsung's **Settings → Display → Continue apps on cover screen**:
  the game moves to the cover screen, or the phone locks and the game waits in the background (the
  session kept, with this setting on).

## Settings that differ from iOS

Everything else is iNilla's ([INILLA.md](INILLA.md#settings)).

| tab | on Android |
|---|---|
| General | Graphics shows each preset's frame rate (Battery saver 30 fps, Balanced 60 fps, High at the panel's rate), and its hint says High is best on a charger. Interface size: automatic per screen (above). **Keep windows clear of the camera cutout** in place of the Dynamic Island row. **Modern lighting (restart)** (off; [above](#modern-lighting)), also on a desktop. Three rows only Android has: **Inner screen: follow the device into portrait** (off), **Stay connected in the background** (on), **Hide the touch controls while a mouse or keyboard is in use** (on). |
| Touch | **This screen** (the screen class), **Copy the phone layout here**, **Tabletop: the game above the hinge, the controls below it** (on). **Reset layout** puts back this screen's own layout. |
| Controller | the pairing hint names Android's Bluetooth settings and USB-C clips. |
| Audio | no silent-switch row: Android has no switch, and Mute or Vibrate mode does not silence the game (the volume keys set its volume). **Let music and podcasts from other apps keep playing** off asks Android for the audio focus, which pauses them. |
| Debug | the console's addresses list Wi-Fi first, then the phone's hotspot or USB tethering, then any other network, a VPN's last (cellular data's never), and **Copy address** takes the first; its hint adds the hotspot and `adb forward`; the session log row says where the log is; **Share session log…**. |
| About | **Vanillish — benilla for Android**, the credit to benilla and iNilla, **Reset all Vanillish settings** (every screen's layout too; the Game Dashboard mode last seen is kept). |
| Server | the server admin menu ([above](#the-server-admin-menu)), on a desktop too; iOS has no Server tab. |

In `benilla-config/inilla/settings.toml` the Android-only settings are `tabletop_split`,
`inner_any_orientation`, `stay_connected` and `hide_touch_with_mouse`, and `last_game_mode` is
the memory behind Game Dashboard (not a switch). `modern_lighting` mirrors benilla's
`modernLighting` in `benilla-config/config.toml`, which decides: the row follows that CVar (a
console change included) and writes it when the player moves it.

## Where the files are

`<package>` is `io.github.klanthier.vanillish` — `io.github.klanthier.vanillish.debug` for a debug
build, which is a separate app with its own folders.

| where | what |
|---|---|
| `Android/data/<package>/files/` | Vanillish's folder in the phone's shared storage — iOS's Documents. A PC sees it over USB (File transfer); the phone's own file managers cannot open `Android/data`. It is also where a copied `Data` folder is dropped ([ANDROID-DATA.md](ANDROID-DATA.md)). |
| `…/files/benilla-config/` | benilla's state, as on every platform: `config.toml`, key bindings, macros, saved variables, `AddOns/`, `Screenshots/`, and `Diagnostics/` (session logs, crash reports). |
| `…/files/benilla-config/inilla/` | iNilla's files: `settings.toml`, `servers.toml`, `controller.toml`, and the touch layouts `touch.toml` (cover), `touch-tablet.toml` (inner), `touch-tabletop.toml` (tabletop). |
| `…/files/.vanillish/Data/` | the game data, once imported or downloaded. Beside the drop box on the same storage, so importing a copied folder is a rename, not a second 5 GB copy. |
| `/data/user/0/<package>/files/secrets.bin` | the saved passwords, access codes and admin codes, sealed (private storage: nothing else can read it); `secrets.bin.unreadable` beside it is one whose key is gone ([Passwords](#passwords)). |
| `/data/user/0/<package>/cache/vanillish/` | caches and temporary files; Android may clear them. |

Shared storage can be away for a moment at launch (right after a system update, while Android
restarts the service behind it). Vanillish then asks again for about three seconds; if it still
does not answer, the setup screen says "Shared storage is not available right now. Close Vanillish
and open it again." in place of the setup, and no download, import or addon sync starts. Nothing
is lost: the folders are where they were for the next launch. Meanwhile the game runs from a
private stand-in (`/data/user/0/<package>/files/Documents/`) that the next launch does not use.

**Uninstalling deletes all of it** — the game data, the settings, the layouts, the SavedVariables —
and so does *Clear storage* in the app's settings. (Uninstalling from the system settings may offer
to keep the app's data; do not count on it.) Updating (an APK installed over the app) keeps
everything, as long as it is signed with the same key ([ANDROID-BUILD.md](ANDROID-BUILD.md#signing)).
Android Auto Backup is off for Vanillish: copy `benilla-config` to a PC to keep it.

## Passwords

**Remember the password** on an account seals it with an AES-256-GCM key kept in the **Android
Keystore** (alias `vanillish.secrets.v1`); the sealed file, `secrets.bin`, sits in private storage.
The key never leaves the Keystore and is never backed up, so a copy of the file is useless
elsewhere. If the key is gone (a reinstall, *Clear storage*), the passwords are forgotten and asked
for again at the next login; the file that can no longer be read is set aside as
`secrets.bin.unreadable`, not deleted, and the next password saved starts a new `secrets.bin`.
Quick connect saves an account's password the same way, and a server's access code beside it
(`data-token:<server id>`); the admin menu, a server's admin code (`admin-code:<server id>`).

A Keystore that is only busy (right after the phone starts, under load) forgets nothing: Vanillish
tries again after a moment, and if it still fails, keeps the file as it is. That login asks for
the password this once, and a password saved meanwhile is not saved. Whenever the Keystore refuses
a password, the account form or the password prompt says "Password not saved: the Android Keystore
refused it", and the launcher asks for it at each login until it is saved.

## The manifest

Every attribute in `android/app/src/main/AndroidManifest.xml` is there for a reason:

| attribute | why |
|---|---|
| `appCategory="game"`, `isGame="true"` | One UI files Vanillish with the games (Game Launcher, Game Booster), and Android 16 keeps honouring a game's landscape lock on a large screen (the inner one). |
| `allowBackup="false"` | 5 GB of game data and Auto Backup's 25 MB quota do not mix, and the Keystore key that seals the passwords is never backed up anyway. |
| `hasFragileUserData="true"` | an uninstall from the system settings offers to keep the app's data. |
| `extractNativeLibs="true"` (and Gradle's `useLegacyPackaging`) | the library is stored compressed in the APK — it keeps its symbol table — and unpacked at install. |
| `android.game_mode_config` → `res/xml/game_mode_config.xml` | Performance and Battery modes declared; no system downscaling, no frame-rate override. |
| `uses-feature android.hardware.vulkan.version` `0x401000` | wgpu's Vulkan backend: Vulkan 1.1 is the floor. The gamepad and the touchscreen are optional features. |
| `INTERNET`, `ACCESS_NETWORK_STATE`, `ACCESS_WIFI_STATE` | the game server, the data host, the remote console; whether the network is metered (`Bridge.networkMetered`), before quick connect starts a download by itself. |
| `VIBRATE` | haptics on the HUD and the radial menu. |
| `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_SPECIAL_USE`, `POST_NOTIFICATIONS` | staying connected: `GameSessionService` and its notification. |
| activity `enableOnBackInvokedCallback="false"` | no predictive back, which targetSdk 36 turns on from Android 16: it would hand every Back key, whatever its source, to the back callback ahead of the Activity, making a pad's Back and a mouse's side button the game's Escape. Without it the Back key and the gesture reach the game as a key, as on Android 12–15. |
| activity `configChanges` (the full list) | folding, unfolding, rotating, a controller or keyboard connecting, DeX, a theme change: none may recreate the Activity — a Bevy app cannot be built twice in one process. |
| `launchMode="singleTask"` | a second launch never makes a second Activity. |
| `screenOrientation="sensorLandscape"`, `resizeableActivity="true"` | landscape either way up (the inner screen may be let into portrait); split screen and DeX windows allowed. |
| `windowSoftInputMode="adjustNothing\|stateHidden"` | the keyboard floats over the game rather than shrinking it (the automatic interface size follows the height). GameActivity asks for the opposite at run time; `VanillishActivity.onSetUpWindow` sets it back. |
| `android.app.lib_name` = `vanillish` | the library GameActivity loads. |
| the FileProvider `${applicationId}.files` | **Share session log…**: hands one file of `benilla-config/Diagnostics` to another app, read-only. |
| `.Relauncher` in its own process `:relauncher` | "Restart Vanillish now": waits for the game's process to end and opens the game in a fresh one. |
| `.GameSessionService`, `foregroundServiceType="specialUse"`, `stopWithTask="true"` | staying connected; no other service type fits a game session, and swiping the game away ends it. |

No storage permission is asked for: the drop box is the app's own folder, and a folder elsewhere
is read through the system's picker.

**One game per process.** A Bevy app is built once per process. When the game quits, the process
ends with it; an Activity that Android ever creates again in a process that already ran the game
starts the game over in a fresh one (`Relauncher`); `VanillishActivity.onDestroy` ends the process
instead of calling GameActivity's teardown, which would wait forever for a winit app to return.
Pipelined rendering is off on Android (`crates/benilla-world/src/boot.rs`): with the render thread
presenting while Android takes the surface away (leaving the screen, a fold), the swapchain is
lost mid-frame, which Bevy cannot recover from.

## Known limits

- **Rumble** is not supported.
- **In the background** the sound is muted, not paused: the mixer keeps running silently. There is
  no wake lock, so with the screen off for a long time the connection can still drop.
- **A dismissed keyboard** (closed with Back or its hide key while the box keeps the focus) does not
  come back on the next tap — iOS has the same gap. Back once more leaves the box (Escape); open
  chat again.
- **With the on-screen keyboard up**, a hardware keyboard's arrows, Home and End move the
  keyboard's cursor, not the game's caret; the text the game gets is still right.
- **Tabletop**: windows taller than the half above the hinge and some addons reach into the deck.
- **Controllers**: a generic HID pad laid out like a DualShock 4 without Android's mapping file
  could read its triggers as the right stick; a Bluetooth keyboard that Android takes for a gamepad
  would have its arrows and Back taken as the d-pad and Options.
- **Copying over USB**: if the PC stamps the files with a time in the future (another time zone,
  a clock off), the folder reads as "still changing" until that time passes.
- **Android 17's local network permission.** Vanillish targets SDK 36, which keeps the local
  network open to it on Android 17 too. An app that targets 37 needs the runtime permission
  `ACCESS_LOCAL_NETWORK` for every connection on the LAN — Connect, the realm, the data host,
  `.local` names, the remote console — and without it they time out. Vanillish does not ask for it
  yet: raising `vanillish.targetSdk` to 37 needs that request, and an answer to a refusal, first.
- **Sign-up behind Docker Desktop or OrbStack**: the server sees every phone at the Docker
  runtime's own gateway address, a private one, so its `INILLA_SIGNUP=lan` (the default) takes a
  sign-up from whoever reaches port 8099, and the per-address limits count every phone together.
  A server reachable from the internet needs `INILLA_DATA_TOKEN`
  ([server/docs/OPERATIONS.md](../server/docs/OPERATIONS.md#accounts-and-gm-levels)).
- **The automatic download**: Android counts most Wi-Fi as unmetered, and a phone's hotspot too
  unless it is marked metered, so on such a hotspot Connect starts 5 GB with no tap.
- **The admin menu**: a boss's or a dungeon's loot that drops one item from a list does not follow
  the item drop rates (vmangos rolls those lists with no rate). Behind Docker Desktop or OrbStack,
  as for sign-up, `INILLA_ADMIN=lan` lets anyone who reaches port 8099 try a code: there the code
  is the only guard, and the wrong-code limit slows no one who keeps guessing, which is why the
  server takes only a random code (the setup's, or `openssl rand -hex 10`).
- **A `.local` name** resolves on Wi-Fi only, not over mobile data or a VPN; the IP the server
  printed always works.
- **iOS**: Vanillish keeps iOS's behaviour by construction (the Android arms are `cfg`-gated, iOS's
  words are asserted by a test), but this port has not been built for iOS.

## What needs a device

Vanillish has been built and checked on Linux only; nothing above has run on a phone yet. The
first device runs should check, on the Fold8 Ultra:

1. `adb shell wm size; adb shell wm density` folded and unfolded: the dp sizes above, the classes
   they give, the camera cut-out on each screen and each landscape rotation (the inner screen's
   camera position is unknown), and the bottom inset with gesture and with 3-button navigation
   (the latter may reserve about 48 dp even with the bar hidden).
2. Folding and unfolding many times, with each *Continue apps on cover screen* option: no new
   Activity, the right layout each time, the surface and the frame-rate vote back.
3. Tabletop: the world drawn above the hinge only (the viewport on Adreno/Vulkan), and the stock
   interface ending exactly at the hinge (`return UIParent:GetBottom()` in the remote console's Lua
   prompt); nameplates and chat bubbles over their units, open bags wrapping below the top, and a
   mouse or the controller's cursor over the deck picking nothing in the world.
4. The back swipe held off beside the stick on One UI 9, and how it meets Samsung's Edge panel.
5. The panel's measured rate in each preset, with Motion smoothness Adaptive and Standard, and
   with Game Booster's settings; Game Dashboard's modes (`adb shell cmd game mode …`), and whether
   One UI offers them.
6. Gboard and Samsung Keyboard: the composing word, passwords, Backspace in an empty box, Return;
   Back on Android 16 and later against 12–15, with predictive back off: the gesture the game's
   Escape, a pad's Back its game menu, a mouse's side button `BUTTON5` alone.
7. Controllers: Xbox, DualSense, Switch Pro, a Backbone on the cover screen, an 8BitDo. The logcat
   line `vanillish: controller <id>: <name> [vvvv:pppp] "<family>", axes …` names the axis layout
   chosen; triggers rest at 0; the LT layer engages; nothing stays held after the shade or a
   fold.
8. Staying connected: start and stop, the permission prompt's timing, the notification; the logout
   30 minutes after the last input with the phone locked, and the service stopping with it.
9. The data: the folder picker opening in Download, a folder picked on a USB drive or a network
   share, a copy resumed after the app was killed, the share sheet, a real 1.12.1 `Data` folder
   passing the archive checks.
10. Saved passwords: a login right after the phone starts, while the Keystore may be busy, keeps
    them (asked for once at worst).
11. DeX and a mouse: pointer positions in a DeX window (its title bar), mouse-look sensitivity
    captured and not, a trackpad's scroll direction and speed.
12. Heat: twenty minutes at High on the inner screen against Balanced, with the perf overlay on.
13. Quick connect against `server/setup-mac.sh`'s server: the IP typed and the `.local` name; the
    download starting by itself on Wi-Fi and waiting for a tap on mobile data; Create account,
    then the first login after **Restart Vanillish now**; a second Connect to the same server
    keeping one server; an access code; Android 17 with targetSdk 36.
14. The server admin menu, against that server, with two characters in the world (two phones, or a
    phone and a desktop client), the owner's on the phone:
    - kill a mob at ×1 and note the XP; set **All experience** to ×2, **Apply**, kill another of
      the same kind and level: twice the XP, and the other character read the announcement in chat;
    - set **Player speed** to ×1.2: both run faster at once, and still after mounting, dismounting,
      a taxi flight (its own speed while it flies), zoning through an instance portal and a relog
      (`.unit speedinfo`, from a GM account, reads the speeds);
    - give 10g to the other character by name, and then to **Everyone online**: "Sent by mail to …",
      and a mailbox opened holds "A gift from the realm" with 10g;
    - **Server defaults**, two taps: every rate back to ×1, the "server ×1" hints and the button
      gone, the message of the day back to the server's;
    - a wrong code: "That is not this server's admin code." and the field again (the eleventh in
      ten minutes: "Too many wrong codes from here lately: …", and the field again); the right
      code still taken after ten wrong ones;
    - `docker compose restart mangosd` with the tab open: "The world server is starting…" with the
      sliders read-only, then up again with the rates as they were;
    - with a hardware keyboard, typing the code or a name in the world moves nobody and opens no
      chat.
