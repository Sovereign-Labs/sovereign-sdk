"""Type stubs for sovereign_web3 module."""

from typing import Dict, Any, List, Optional

class Serializer:
    """Schema-based transaction serializer for Sovereign rollups."""

    def __init__(self, schema_json: str) -> None:
        """Create serializer from JSON schema string.

        Args:
            schema_json: JSON schema as string

        Raises:
            ValueError: If schema JSON is invalid
        """
        ...

    @classmethod
    def from_url(cls, url: str) -> "Serializer":
        """Create serializer by fetching schema from URL.

        Args:
            url: URL to fetch schema from

        Returns:
            New Serializer instance

        Raises:
            ValueError: If URL is unreachable or returns invalid schema
        """
        ...

    def chain_hash(self) -> bytes:
        """Get the chain hash.

        Returns:
            Chain hash as bytes

        Raises:
            ValueError: If chain hash cannot be computed
        """
        ...

    def serialize_unsigned_tx(self, unsigned_tx: "UnsignedTransaction") -> bytes:
        """Serialize an unsigned transaction.

        Args:
            unsigned_tx: Transaction to serialize

        Returns:
            Serialized transaction bytes

        Raises:
            ValueError: If serialization fails
        """
        ...

    def serialize_tx(self, tx: "Transaction") -> bytes:
        """Serialize a signed transaction.

        Args:
            tx: Signed transaction to serialize

        Returns:
            Serialized transaction bytes

        Raises:
            ValueError: If serialization fails
        """
        ...

class TxDetails:
    """Transaction details including fees and gas limits."""

    def __init__(
        self,
        chain_id: int,
        max_fee: int = ...,
        max_priority_fee_bips: int = ...,
        gas_limit: Optional[List[int]] = None,
    ) -> None:
        """Create transaction details.

        Args:
            chain_id: Chain identifier
            max_fee: Maximum fee (default: DEFAULT_MAX_FEE)
            max_priority_fee_bips: Maximum priority fee in basis points (default: DEFAULT_MAX_PRIORITY_FEE_BIPS)
            gas_limit: Optional gas limit per module
        """
        ...

    @property
    def chain_id(self) -> int:
        """Chain identifier."""
        ...

    @chain_id.setter
    def chain_id(self, value: int) -> None:
        """Set chain identifier."""
        ...

    @property
    def max_fee(self) -> int:
        """Maximum fee."""
        ...

    @max_fee.setter
    def max_fee(self, value: int) -> None:
        """Set maximum fee."""
        ...

    @property
    def max_priority_fee_bips(self) -> int:
        """Maximum priority fee in basis points."""
        ...

    @max_priority_fee_bips.setter
    def max_priority_fee_bips(self, value: int) -> None:
        """Set maximum priority fee in basis points."""
        ...

class UniquenessData:
    """Transaction uniqueness data (nonce or generation)."""

    @staticmethod
    def nonce(nonce: int) -> "UniquenessData":
        """Create nonce-based uniqueness data.

        Args:
            nonce: Nonce value

        Returns:
            UniquenessData instance
        """
        ...

    @staticmethod
    def generation(generation: int) -> "UniquenessData":
        """Create generation-based uniqueness data.

        Args:
            generation: Generation value

        Returns:
            UniquenessData instance
        """
        ...

    @staticmethod
    def default() -> "UniquenessData":
        """Create default uniqueness data.

        Returns:
            Default UniquenessData instance

        Raises:
            ValueError: If default uniqueness cannot be created
        """
        ...

class UnsignedTransaction:
    """Unsigned transaction with runtime call and details."""

    def __init__(
        self,
        runtime_call: Dict[str, Any],
        details: TxDetails,
        uniqueness: Optional[UniquenessData] = None,
    ) -> None:
        """Create unsigned transaction.

        Args:
            runtime_call: Runtime call data as dictionary
            details: Transaction details
            uniqueness: Optional uniqueness data (defaults to default uniqueness)

        Raises:
            ValueError: If runtime_call cannot be converted to JSON or uniqueness creation fails
        """
        ...

    def bytes_for_signing(self, serializer: Serializer) -> bytes:
        """Get transaction bytes for signing.

        Args:
            serializer: Serializer to use

        Returns:
            Bytes to be signed

        Raises:
            ValueError: If signing bytes cannot be generated
        """
        ...

    def to_signed(self, pub_key: bytes, signature: bytes) -> "Transaction":
        """Convert to signed transaction.

        Args:
            pub_key: Public key bytes
            signature: Signature bytes

        Returns:
            Signed Transaction instance
        """
        ...

class Transaction:
    """Signed transaction ready for submission."""

    ...

# Module constants
DEFAULT_MAX_FEE: int
DEFAULT_MAX_PRIORITY_FEE_BIPS: int
