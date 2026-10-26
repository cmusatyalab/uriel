/*
 * The producer "hello-app": publishes each of its arguments on the flow "hello", and prints the
 * engine's result for each. Start the server, the engine and the client first.
 *
 *     ./producer hello uriel
 */
#include <stdio.h>
#include <string.h>
#include <uriel/uriel.h>

#define PRODUCER_NAME "hello-app"
#define FLOW_NAME "hello"
#define MAX_INPUT_SIZE 1024
/* How long to wait for each result. */
#define RESULT_TIMEOUT_MS 2000

#define CHECK(call)                                                 \
    do {                                                            \
        if ((call) != URIEL_STATUS_T_OK) {                          \
            fprintf(stderr, "%s: %s\n", #call, uriel_last_error()); \
            return 1;                                               \
        }                                                           \
    } while (0)

int main(int argc, char **argv) {
    uriel_producer_t *producer;
    CHECK(uriel_producer_create(PRODUCER_NAME, NULL, MAX_INPUT_SIZE, &producer));

    /* What happens to the producer's inputs. Subscribe before publishing, to miss nothing. */
    uriel_subscription_t *events;
    CHECK(uriel_subscribe_producer(PRODUCER_NAME, &events));

    for (int i = 1; i < argc; i++) {
        /* Write the input into a loaned buffer, then publish it on the flow. */
        uint8_t *buf;
        uint64_t seq;
        size_t len = strnlen(argv[i], MAX_INPUT_SIZE);  /* the buffer holds MAX_INPUT_SIZE bytes */
        CHECK(uriel_producer_new_input(producer, &buf));
        memcpy(buf, argv[i], len);
        CHECK(uriel_producer_publish_input(producer, len, FLOW_NAME, &seq));

        /* Wait for the engine's result about this input. */
        uriel_event_t event;
        do {
            CHECK(uriel_subscription_wait(events, RESULT_TIMEOUT_MS));
            CHECK(uriel_subscription_poll(events, &event));
        } while (event.kind != URIEL_EVENT_KIND_T_RESULT || event.id.seq != seq);
        printf("%s replied: %.*s\n", event.engine, (int)event.result_len,
               (const char *)event.result);
    }

    uriel_subscription_destroy(events);
    uriel_producer_destroy(producer);
    return 0;
}
