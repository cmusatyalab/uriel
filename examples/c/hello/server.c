/*
 * The server: serves the engine "hello-upper" on this machine, until Ctrl-C.
 *
 *     ./server
 */
#include <signal.h>
#include <stdio.h>
#include <unistd.h>
#include <uriel/uriel.h>

#define SERVER_NAME "hello-server"
#define ENGINE_NAME "hello-upper"

static volatile sig_atomic_t stopping = 0;

static void stop(int signal) {
    (void)signal;
    stopping = 1;
}

int main(void) {
    signal(SIGINT, stop);
    signal(SIGTERM, stop);

    /* NULL options: a server for clients on this machine only. */
    uriel_server_t *server;
    if (uriel_server_create(SERVER_NAME, NULL, &server) != URIEL_STATUS_T_OK) {
        fprintf(stderr, "server: %s\n", uriel_last_error());
        return 1;
    }
    /* The engine may start before or after this. */
    if (uriel_server_add_engine(server, ENGINE_NAME) != URIEL_STATUS_T_OK) {
        fprintf(stderr, "add engine: %s\n", uriel_last_error());
        uriel_server_destroy(server);
        return 1;
    }
    printf("server %s serves %s; Ctrl-C to stop\n", SERVER_NAME, ENGINE_NAME);

    /* The server works on its own thread; this one only waits for Ctrl-C. */
    while (!stopping) {
        pause();
    }
    uriel_server_destroy(server);
    return 0;
}
