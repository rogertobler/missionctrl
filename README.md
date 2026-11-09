# MissionCtrl for herdr

MissionCtrl is a mobile app to watch and control [herdr](https://github.com/herdrdev/herdr)
sessions on several remote servers at once: workspaces, tabs, panes and the status of the AI
agents running in them. Android first, iOS later.

All a server needs is a running herdr and SSH access. The app talks to herdr's Unix sockets
directly over SSH; nothing is installed on the server.

MissionCtrl is an independent project. It is not made, maintained or endorsed by the herdr
project; "herdr" here only names the software the app works with.

## Status

In development. The platform-independent core (Rust) connects to several herdr servers over SSH,
keeps their state live and reconnects by itself after a dropped connection. The Android app
(Kotlin, Jetpack Compose) uses the core through UniFFI:

- answer waiting agents right away: tap a menu option, type or dictate
- read the coloured history, or use a pane as a real terminal (TUI)
- a key bar with every key and Ctrl/Alt/Shift to combine
- paste images and upload files
- create, move and close tabs and workspaces
- notifications in the background, without a push service
- several servers and SSH keys
- dictation with on-device recognition or a local Whisper model (whisper.cpp)
- a demo mode with sample agents, to try the app without a server

An iOS app (SwiftUI) will follow.

## Layout

```text
core/                     Rust core (Cargo workspace)
  crates/mc-core/         SSH, herdr socket API, one runtime per server, several servers together
  crates/mc-cli/          developer tool `mc` to test against real servers
  crates/mc-ffi/          UniFFI interface for Kotlin (and later Swift)
  crates/mc-voice/        speech recognition on the device with whisper.cpp
  crates/uniffi-bindgen/  generates the Kotlin bindings
  hosts.example.json      template for the server list of `mc`
android/                  Android app (Gradle, Jetpack Compose)
```

## Build and test

Requires Rust (stable).

```bash
cd core
cargo build
cargo test
```

The `mc` tool reads its server list from `~/.config/missionctrl/hosts.json`
(template: `core/hosts.example.json`):

```bash
./target/debug/mc hosts              # configured servers
./target/debug/mc ping <host>        # herdr version and socket
./target/debug/mc watch              # live view of all servers and agents
```

### Android app

Requires JDK 17+, the Android SDK (platform 37), the Android NDK, CMake, `cargo-ndk` and the Rust
target `aarch64-linux-android`. The Gradle build builds the Rust core and the bindings itself; it
finds the NDK through `ANDROID_NDK_HOME`.

```bash
cd android
echo "sdk.dir=$HOME/Android/Sdk" > local.properties
./gradlew assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

The app runs on Android 10 and later (arm64).

## SSH keys

MissionCtrl does not create a key by itself. When you add your first server, or under
Settings › SSH keys, you create a new key (Ed25519) or import one you already have (OpenSSH, PEM
or PuTTY, from the clipboard or a file, also with a passphrase). The app shows the public part
of a new key; it goes into `~/.ssh/authorized_keys` on the server.

Private keys are stored encrypted with a key from the Android Keystore and are never backed up.
A key can additionally be protected with biometrics or the device PIN; the app then asks for it
before it connects. A key can be exported as a file protected with a passphrase.

## Privacy

MissionCtrl collects no data. The app contains no analytics, tracking, crash reporting or
advertising libraries, does not use Google Play Services and talks to no one but the servers you
add yourself. The one exception: if you use Whisper, the chosen speech model is downloaded from
huggingface.co when you tap to download it. App data is excluded from Android backup (Google
Drive, device transfer). The keyboard does not learn from answers, and the fonts ship with the
app.

Speech recognition: "On device" and Whisper keep the audio on the phone. If you choose another
installed recognition service, its privacy terms apply; some send the audio to their provider.
The app points this out when you choose one.

## Languages

The app is available in English (the default) and German. Choose under Settings › Language
("System" follows Android's language), or from Android 13 on in Android's app settings.

A further language needs just one file: copy `android/app/src/main/res/values/strings.xml` to
`values-xx/strings.xml` (`xx` = language code, e.g. `values-fr`) and translate the texts. Leave
out entries with `translatable="false"`. The build adds the language to the list by itself.

## Uploaded files

Pasted images and uploaded files are stored on the server under `~/.cache/missionctrl/uploads/`
(readable only by your own user), and their path goes into the input. Cleaning up is up to you.

## Third-party software

Every library, font and speech model is listed with its licence in the app under
Settings › About › Open-source licenses. The list
(`android/app/src/main/assets/licenses/third_party.json`) is regenerated from the Rust and Gradle
dependencies with `python3 android/scripts/third_party_licenses.py` when they change. The fonts
IBM Plex Sans and IBM Plex Mono are licensed under the SIL Open Font License 1.1.

## Licence

© 2026 Inscribe GmbH. MissionCtrl is free software under the GNU General Public License,
version 3 or (at your option) any later version (`GPL-3.0-or-later`). The full text is in
[LICENSE](LICENSE).

The versions in the app stores (Google Play, later the App Store) are published by Inscribe GmbH.
They cost a little but contain the same code; buying them supports further development.

## Contributing

Bug reports, ideas and pull requests are welcome. Contributions are subject to the terms in
[CONTRIBUTING.md](CONTRIBUTING.md): they are licensed under the GPL, and Inscribe GmbH may also
use them in the store versions.

To report a security problem, please see [SECURITY.md](SECURITY.md) instead of opening an issue.
