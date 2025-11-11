import pytest
import sovereign_web3


def test_sum_as_string():
    assert sovereign_web3.sum_as_string(1, 1) == "2"
