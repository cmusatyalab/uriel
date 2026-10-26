/*
 * The engine "hello-upper": upper-cases each input, and reports it as its result, until Ctrl-C.
 *
 *     ./engine
 */
#include <ctype.h>
#include <signal.h>
#include <stdio.h>
#include <uriel/uriel.h>

#define ENGINE_NAME "hello-upper"
#define MAX_OUTPUT_SIZE 1024
/* How long each wait lasts, so that Ctrl-C is noticed. */
#define WAIT_MS 100

static volatile sig_atomic_t stopping = 0;

static void stop(int signal) {
    (void)signal;
    stopping = 1;
}

int main(void) {
    signal(SIGINT, stop);
    signal(SIGTERM, stop);

    /* NULL result type: the results are plain text. */
    uriel_engine_t *engine;
    if (uriel_engine_create(ENGINE_NAME, NULL, MAX_OUTPUT_SIZE, &engine) != URIEL_STATUS_T_OK) {
        fprintf(stderr, "engine: %s\n", uriel_last_error());
        return 1;
    }
    printf("engine %s is ready; Ctrl-C to stop\n", ENGINE_NAME);

    while (!stopping) {
        /* ERR_AGAIN means nothing came in time. */
        if (uriel_engine_wait(engine, WAIT_MS) != URIEL_STATUS_T_OK) {
            continue;
        }
        uriel_input_t input;
        if (uriel_engine_poll_input(engine, &input) != URIEL_STATUS_T_OK) {
            continue;
        }

        /* input.buf stays valid until the outcome below. */
        uint8_t result[MAX_OUTPUT_SIZE];
        size_t len = input.len < MAX_OUTPUT_SIZE ? input.len : MAX_OUTPUT_SIZE;
        for (size_t i = 0; i < len; i++) {
            result[i] = (uint8_t)toupper(input.buf[i]);
        }
        printf("input %llu from %s: %.*s\n", (unsigned long long)input.id.seq, input.id.producer,
               (int)input.len, (const char *)input.buf);

        /* The outcome: drop, since nothing comes after this engine, with the result attached. */
        if (uriel_engine_drop(engine, result, len) != URIEL_STATUS_T_OK) {
            fprintf(stderr, "drop: %s\n", uriel_last_error());
        }
    }
    uriel_engine_destroy(engine);
    return 0;
}
