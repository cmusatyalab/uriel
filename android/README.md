# Uriel for Android

An AAR that lets an Android app send inputs to a remote Uriel server over QUIC, and read what happened to them. It has the `Client`, `Producer` and `Subscription` of Uriel's API; there is no local server or engine on the phone, so a flow's engines all run on the remote server (`location = "remote"`).

| Directory  | What it is                                                                                  |
| ---------- | ------------------------------------------------------------------------------------------- |
| `uniffi/`  | The Rust crate. It wraps `uriel` with the `in-process` feature, and UniFFI exports it.      |
| `lib/`     | The Gradle module that builds the AAR: the Rust library per ABI, and the Kotlin bindings.   |
| `example/` | An example app, in Java.                                                                    |

Kotlin and Java apps use the same AAR. The classes are in the package `uniffi.uriel_android`, and `Client`, `Producer` and `Subscription` are `AutoCloseable`. Numbers are `Long`, so that Java can call them. Inputs and results are copied to and from `ByteArray`s.

## Build

You need a JDK (17 or newer, not just a JRE; Android Studio's bundled one works), the Android SDK with NDK 30.0.16248370, and Rust with cargo-ndk:

```sh
rustup target add aarch64-linux-android x86_64-linux-android
cargo install cargo-ndk
```

Then:

```sh
cd android
export JAVA_HOME=~/android-studio/jbr      # a JDK, if your default java is only a JRE
./gradlew :lib:assembleRelease             # lib/build/outputs/aar/lib-release.aar
./gradlew :example:assembleDebug           # example/build/outputs/apk/debug/example-debug.apk
```

Gradle runs `cargo ndk` for `arm64-v8a` and `x86_64`, and generates the Kotlin bindings from the library it built, so the two always match. 32-bit ARM is not supported, because iceoryx2 does not support it.

Android Studio opens the `android/` directory as a project.

## Use the AAR

```java
RemoteServer server = new RemoteServer("192.0.2.1:4433", certificateDer, null);
Client client = new Client("my-app", server);
Producer producer = new Producer("camera", "app.Frame", 4 << 20);
Subscription events = client.addProducerWithEvents("camera"); // before the first input
client.addFlow("detect", spec);                                // TOML; its engines are remote

long seq = producer.send(frameBytes);
Event event = events.next(500);                                // null if nothing came in 500 ms
```

`certificateDer` is the server's `certificate.der`; the client trusts that certificate only. `Subscription.next` blocks, so call it off the main thread. All three objects may be used from any thread.

## Run the example

1. On a computer, start a server and an engine. This is the C example's engine (see `examples/c/hello`), served by `uriel-server`:

   ```sh
   cargo run -p uriel-server -- --identity-dir /tmp/uriel-identity --engine hello-upper
   ./engine            # in examples/c/hello, after `make`
   ```

2. Copy the server's certificate into the app:

   ```sh
   cp /tmp/uriel-identity/certificate.der example/src/main/assets/server-certificate.der
   ```

3. Run the app. In the emulator, `10.0.2.2` is the computer; on a phone, use the computer's address. Tap Connect, then Send.
