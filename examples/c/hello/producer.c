/*
 * The producer "hello-app": publishes each of its arguments. It never reads results: the client
 * that takes its inputs does (see client.c). Start the server, the engine and the client first.
 *
 *     ./producer hello uriel
 */
#include <inttypes.h>
#include <stdio.h>
#include <string.h>
#include <time.h>
#include <uriel/uriel.h>

#define PRODUCER_NAME "hello-app"
#define MAX_INPUT_SIZE 1024
/* The client keeps only a producer's newest input, and a producer that exits at once can take its
 * last input with it. So give each input time to be taken. */
#define PAUSE_AFTER_INPUT_NS 300000000L

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

    for (int i = 1; i < argc; i++) {
        /* Write the input into a loaned buffer, then publish it. */
        uint8_t *buf;
        uint64_t seq;
        size_t len = strnlen(argv[i], MAX_INPUT_SIZE);  /* the buffer holds MAX_INPUT_SIZE bytes */
        CHECK(uriel_producer_new_input(producer, &buf));
        memcpy(buf, argv[i], len);
        CHECK(uriel_producer_publish_input(producer, len, &seq));

        printf("published %s as input %" PRIu64 "\n", argv[i], seq);
        const struct timespec pause = {.tv_sec = 0, .tv_nsec = PAUSE_AFTER_INPUT_NS};
        nanosleep(&pause, NULL);
    }

    uriel_producer_destroy(producer);
    return 0;
}
