// Command hello shows Uriel's API in the fewest steps: an app sends text to an engine that
// upper-cases it, and reads the result back.
//
//	go run ./examples/go/hello
//
// Everything runs in one goroutine, one step after another. In a real deployment the server, the
// engine and the app are separate processes.
package main

import (
	"context"
	"fmt"
	"log"
	"strings"
	"time"

	uriel "github.com/cmusatyalab/uriel/go"
)

const flow = `
[[producers]]
name = "hello-app"
tokens = 1

[[engines]]
name = "hello-upper"
location = "local"

[[edges]]
from = "hello-app"
to = "hello-upper"
timeout = "1s"
`

func main() {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	// 1. A server, and an engine it serves.
	server, err := uriel.NewServer("hello-server")
	check(err)
	defer server.Close()
	engine, err := uriel.NewEngine("hello-upper", "", 1024)
	check(err)
	defer engine.Close()
	check(server.AddEngine("hello-upper"))

	// 2. A producer for the app's inputs, and a subscription to what happens to them.
	producer, err := uriel.NewProducer("hello-app", "", 1024)
	check(err)
	defer producer.Close()
	events, err := uriel.SubscribeProducer("hello-app")
	check(err)
	defer events.Close()

	// 3. A client that sends the producer's inputs through the flow.
	client, err := uriel.NewClient("hello-client", "hello-server")
	check(err)
	defer client.Close()
	check(client.AddProducer("hello-app"))
	check(client.AddFlow("hello", flow))

	// 4. The app sends an input.
	_, err = producer.Send("hello", []byte("hello, uriel"))
	check(err)

	// 5. The engine gets it, and reports its result.
	input, err := engine.Next(ctx)
	check(err)
	check(engine.Drop([]byte(strings.ToUpper(string(input.Data)))))

	// 6. The app reads the result.
	for {
		event, err := events.Next(ctx)
		check(err)
		if event.Kind == uriel.EventResult {
			fmt.Printf("%s replied: %s\n", event.Engine, event.Result)
			return
		}
	}
}

func check(err error) {
	if err != nil {
		log.Fatal(err)
	}
}
