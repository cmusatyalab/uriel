/*
 * The client: takes the inputs of the producer "hello-app", and sends them through the flow
 * "hello" to the engine "hello-upper", until Ctrl-C. Start it after the server, and before the
 * producer publishes: nothing takes an input published before the flow exists.
 *
 *     ./client
 */
#include <signal.h>
#include <stdio.h>
#include <unistd.h>
#include <uriel/uriel.h>

#define CLIENT_NAME "hello-client"
#define SERVER_NAME "hello-server"
#define PRODUCER_NAME "hello-app"
#define FLOW_NAME "hello"

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
    /* First the producer, then the flow that uses it. */
    if (uriel_client_add_producer(client, PRODUCER_NAME) != URIEL_STATUS_T_OK ||
        uriel_client_add_flow(client, FLOW_NAME, FLOW_SPEC) != URIEL_STATUS_T_OK) {
        fprintf(stderr, "flow: %s\n", uriel_last_error());
        uriel_client_destroy(client);
        return 1;
    }
    printf("client %s routes %s through the flow %s; Ctrl-C to stop\n", CLIENT_NAME,
           PRODUCER_NAME, FLOW_NAME);

    /* The client works on its own thread; this one only waits for Ctrl-C. */
    while (!stopping) {
        pause();
    }
    /* Destroying the client also removes its flow from the server. */
    uriel_client_destroy(client);
    return 0;
}
