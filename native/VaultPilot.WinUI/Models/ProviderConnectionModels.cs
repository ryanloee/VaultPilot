using System.Text.Json.Serialization;

namespace VaultPilot.WinUI.Models;

// ── #3480: Provider connection test DTOs ────────────────────────
// These mirror the Rust backend's CheckProviderConnectionParams and
// ProviderConnectionResult (see src/bin/vaultpilot-agent.rs).

/// <summary>
/// Request payload sent to the Rust backend's `checkProviderConnection`
/// IPC method. Field names are camelCase to match JSON-RPC conventions.
/// </summary>
public sealed class ProviderConnectionRequest
{
    public string ApiBase { get; set; } = string.Empty;
    public string ApiKey { get; set; } = string.Empty;
    public string ProviderType { get; set; } = "openai";
    public string? Model { get; set; }
    public ulong? TimeoutMs { get; set; }
}

/// <summary>
/// Response payload returned by the Rust backend after probing the
/// configured provider's /models (or /api/tags for Ollama) endpoint.
/// </summary>
public sealed class ProviderConnectionResult
{
    public bool Ok { get; set; }

    [JsonPropertyName("status")]
    public ushort Status { get; set; }

    [JsonPropertyName("error")]
    public string? Error { get; set; }

    [JsonPropertyName("probeUrl")]
    public string? ProbeUrl { get; set; }
}
