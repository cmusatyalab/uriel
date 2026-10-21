"""The errors Uriel raises. Each says what went wrong."""


class UrielError(Exception):
    """A failed call into Uriel."""


class InvalidArgumentError(UrielError, ValueError):
    """An argument was invalid, e.g. an unknown name or a malformed flow."""


class SizeExceededError(UrielError, ValueError):
    """A payload is larger than its maximum size."""


class AgainError(UrielError):
    """Nothing is available yet. The bindings turn it into None or False, so it seldom surfaces."""


class InternalError(UrielError, RuntimeError):
    """Uriel failed internally, e.g. in its shared memory transport."""


class WrongThreadError(UrielError, RuntimeError):
    """A producer, engine or subscription was used from a thread other than the one that made it."""


class ClosedError(UrielError, ValueError):
    """The handle was already closed."""
