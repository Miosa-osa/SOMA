"""SOMA local-alpha protocol planning and response validation."""

from .cli import build_cli_calls
from .mcp import build_mcp_calls
from .model import (
    MACOS_DNS_SERVER,
    CliCall,
    McpCall,
    ProtocolValidationError,
    ResponseEvidence,
)
from .validation import validate_cli_response, validate_mcp_response

__all__ = (
    "MACOS_DNS_SERVER",
    "CliCall",
    "McpCall",
    "ProtocolValidationError",
    "ResponseEvidence",
    "build_cli_calls",
    "build_mcp_calls",
    "validate_cli_response",
    "validate_mcp_response",
)
