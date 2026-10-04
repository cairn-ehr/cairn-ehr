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


def test_the_default_bulk_threshold_is_set_from_the_measured_break_even():
    # Ruling R11: break-even was ~83 charts at 2 000 records and ~15 at 10 000 (Task 7, M3 Max);
    # 30 keeps the worst case within ~2x either way across that range.
    from cairn_matcher.pipeline.worker import Settings

    assert Settings().bulk_threshold == 30
    assert choose_mode(30, Settings().bulk_threshold) is Mode.PER_CHART
    assert choose_mode(31, Settings().bulk_threshold) is Mode.SWEEP


def test_a_sweep_counts_as_completed_work_unless_every_attempt_failed():
    # R4 review N3: a sweep with nothing to score is completed work (the queue was handled); one
    # in which every attempted pair raised is not, or a systematic failure would read "running".
    from cairn_matcher.pipeline.worker_plan import sweep_completed_work

    assert sweep_completed_work(scored=0, failed=0) is True
    assert sweep_completed_work(scored=3, failed=2) is True
    assert sweep_completed_work(scored=0, failed=5) is False
