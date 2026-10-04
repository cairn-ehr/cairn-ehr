"""R4 Task 4: the worker's pure decisions."""

from cairn_matcher.pipeline.worker_plan import Mode, RetryBook, choose_mode


def test_a_backlog_over_the_threshold_is_swept_once():
    assert choose_mode(0, 500) is Mode.PER_CHART
    assert choose_mode(500, 500) is Mode.PER_CHART
    assert choose_mode(501, 500) is Mode.SWEEP


def test_a_failing_chart_is_held_for_the_backoff_then_released():
    book = RetryBook(backoff_s=300.0)
    book.failed("p", now=1000.0)
    assert book.held(1000.0) == ["p"]
    assert book.held(1299.9) == ["p"]
    assert book.held(1300.1) == []


def test_success_clears_a_hold():
    book = RetryBook(backoff_s=300.0)
    book.failed("p", now=0.0)
    book.succeeded("p")
    assert book.held(1.0) == []


def test_a_throttle_is_due_at_once_then_at_most_once_per_interval():
    from cairn_matcher.pipeline.worker_plan import Throttle

    t = Throttle(interval_s=30.0)
    assert t.due(100.0) is True        # the first call is always due (a sweep's opening stamp)
    assert t.due(100.0) is False
    assert t.due(129.9) is False
    assert t.due(130.0) is True        # a full interval since the last due call
    assert t.due(131.0) is False       # the interval restarts from the last due call
