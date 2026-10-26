/*
 * The client: takes the inputs of the producer "hello-app", sends them through the flow "hello" to
 * the engine "hello-upper", and prints the engine's result for each, until Ctrl-C. Start it after
 * the server, and before the producer publishes: nothing takes an input published before the flow
 * exists.
 *
 *     ./client
 */
#include <inttypes.h>
#include <signal.h>
#include <stdio.h>
#include <uriel/uriel.h>

#define CLIENT_NAME "hello-client"
#define SERVER_NAME "hello-server"
#define PRODUCER_NAME "hello-app"
#define FLOW_NAME "hello"
/* How long to wait for an event before checking for Ctrl-C. */
#define EVENT_WAIT_MS 200

/* One edge, from the producer to the engine on the local server. */
static const char *FLOW_SPEC =
    "[[producers]]\n"
    "name = \"" PRODUCER_NAME "\"\n"
    "[[engines]]\n"
    "name = \"hello-upper\"\n"
    "location = \"local\"\n"
    "[[edges]]\n"
    "from = \"" PRODUCER_NAME "\"\n"
    "to = \"hello-upper\"\n"
    "timeout = \"1s\"\n";

static volatile sig_atomic_t stopping = 0;

static void stop(int signal) {
    (void)signal;
    stopping = 1;
}

int main(void) {
    signal(SIGINT, stop);
    signal(SIGTERM, stop);

    /* NULL options: no remote server; the flow's engine is on the local server. */
    uriel_client_t *client;
    if (uriel_client_create(CLIENT_NAME, SERVER_NAME, NULL, &client) != URIEL_STATUS_T_OK) {
        fprintf(stderr, "client: %s\n", uriel_last_error());
        return 1;
    }
    /* First the producer, then the flow that uses it. The client publishes the events about the
     * producer's inputs, and add_producer gives a subscription to them, which misses none. */
    uriel_subscription_t *events;
    if (uriel_client_add_producer(client, PRODUCER_NAME, &events) != URIEL_STATUS_T_OK ||
        uriel_client_add_flow(client, FLOW_NAME, FLOW_SPEC) != URIEL_STATUS_T_OK) {
        fprintf(stderr, "flow: %s\n", uriel_last_error());
        uriel_client_destroy(client);
        return 1;
    }
    printf("client %s routes %s through the flow %s; Ctrl-C to stop\n", CLIENT_NAME,
           PRODUCER_NAME, FLOW_NAME);

    /* The client works on its own thread; this one reads the events. */
    while (!stopping) {
        uriel_status_t status = uriel_subscription_wait(events, EVENT_WAIT_MS);
        if (status == URIEL_STATUS_T_ERR_AGAIN) {
            continue;
        }
        uriel_event_t event;
        while (status == URIEL_STATUS_T_OK) {
            status = uriel_subscription_poll(events, &event);
            if (status == URIEL_STATUS_T_OK && event.kind == URIEL_EVENT_KIND_T_RESULT) {
                printf("input %" PRIu64 ": %s replied: %.*s\n", event.id.seq, event.engine,
                       (int)event.result_len, (const char *)event.result);
            }
        }
        if (status != URIEL_STATUS_T_ERR_AGAIN) {
            fprintf(stderr, "events: %s\n", uriel_last_error());
            break;
        }
    }
    uriel_subscription_destroy(events);
    /* Destroying the client also removes its flow from the server. */
    uriel_client_destroy(client);
    return 0;
}
