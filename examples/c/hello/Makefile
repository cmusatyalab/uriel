# Builds the C example against liburiel, which pkg-config finds (see the README's Install section).
CFLAGS ?= -std=c11 -O2 -Wall -Wextra
CPPFLAGS += -D_POSIX_C_SOURCE=200809L $(shell pkg-config --cflags uriel)
LDLIBS += $(shell pkg-config --libs uriel)

PROGRAMS := server engine client producer

.PHONY: all clean
all: $(PROGRAMS)

clean:
	rm -f $(PROGRAMS)
