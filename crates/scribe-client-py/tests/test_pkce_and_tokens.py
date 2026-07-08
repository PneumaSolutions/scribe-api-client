import string
from scribe_client import PkceChallenge, TokenSet


def test_pkce_challenge_verifier_matches_rfc7636_charset_and_length():
    pkce = PkceChallenge()
    allowed = set(string.ascii_letters + string.digits + "-._~")
    assert len(pkce.verifier) == 43
    assert set(pkce.verifier) <= allowed


def test_two_generated_challenges_differ():
    a = PkceChallenge()
    b = PkceChallenge()
    assert a.verifier != b.verifier
    assert a.challenge != b.challenge


def test_token_set_round_trips_all_fields():
    tokens = TokenSet("at-123", "rt-456", 1_700_000_000.0)
    assert tokens.access_token == "at-123"
    assert tokens.refresh_token == "rt-456"
    assert tokens.expires_at == 1_700_000_000.0


def test_token_set_defaults_optional_fields_to_none():
    tokens = TokenSet("at-123")
    assert tokens.refresh_token is None
    assert tokens.expires_at is None
