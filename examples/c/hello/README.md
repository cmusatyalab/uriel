# Hello, in C

A producer sends words through Uriel to an engine that upper-cases them, and prints the replies. Each part is its own program, as in a real deployment:

| Program    | What it does                                                               |
| ---------- | -------------------------------------------------------------------------- |
| `server`   | Serves the engine `hello-upper` on this machine.                           |
| `engine`   | Upper-cases each input, and reports it as its result.                      |
| `client`   | Takes the producer's inputs, and sends them through the flow `hello`.      |
| `producer` | Publishes each word given to it, and prints the engine's reply.           |

## Build

Install liburiel so that pkg-config finds it (see Install in the [README](../../../README.md)), then:

```sh
export PKG_CONFIG_PATH=~/.local/lib/pkgconfig   # where you installed liburiel
make
```

## Run

Start each program in its own terminal, in this order:

```sh
./server
./engine
./client
./producer hello uriel
```

The producer prints:

```
hello-upper replied: HELLO
hello-upper replied: URIEL
```

Run `./producer` again with other words as often as you like. Stop the other three with Ctrl-C.

The client must be running before the producer publishes: nothing takes an input published before its flow exists, and the producer then gives up after 2 seconds.
