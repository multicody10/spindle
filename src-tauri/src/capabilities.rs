use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySupport {
    Supported,
    ProviderDependent,
    NotImplemented,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilities {
    pub provider_id: String,
    pub display_name: String,
    pub protocol: String,
    pub streaming: CapabilitySupport,
    pub cancellation: CapabilitySupport,
    pub usage_metrics: CapabilitySupport,
    pub tool_calling: CapabilitySupport,
    pub structured_output: CapabilitySupport,
    pub parallel_sessions: CapabilitySupport,
    pub managed_by_spindle: bool,
}

pub fn registry() -> Vec<ProviderCapabilities> {
    vec![
        ProviderCapabilities {
            provider_id: "ollama".into(),
            display_name: "Ollama".into(),
            protocol: "Ollama chat streaming API".into(),
            streaming: CapabilitySupport::Supported,
            cancellation: CapabilitySupport::Supported,
            usage_metrics: CapabilitySupport::Supported,
            tool_calling: CapabilitySupport::ProviderDependent,
            structured_output: CapabilitySupport::NotImplemented,
            parallel_sessions: CapabilitySupport::ProviderDependent,
            managed_by_spindle: false,
        },
        ProviderCapabilities {
            provider_id: "lm-studio".into(),
            display_name: "LM Studio".into(),
            protocol: "OpenAI-compatible chat completions".into(),
            streaming: CapabilitySupport::Supported,
            cancellation: CapabilitySupport::Supported,
            usage_metrics: CapabilitySupport::ProviderDependent,
            tool_calling: CapabilitySupport::ProviderDependent,
            structured_output: CapabilitySupport::NotImplemented,
            parallel_sessions: CapabilitySupport::ProviderDependent,
            managed_by_spindle: false,
        },
        ProviderCapabilities {
            provider_id: "llama-cpp".into(),
            display_name: "Spindle llama.cpp".into(),
            protocol: "OpenAI-compatible chat completions".into(),
            streaming: CapabilitySupport::Supported,
            cancellation: CapabilitySupport::Supported,
            usage_metrics: CapabilitySupport::ProviderDependent,
            tool_calling: CapabilitySupport::ProviderDependent,
            structured_output: CapabilitySupport::NotImplemented,
            parallel_sessions: CapabilitySupport::Supported,
            managed_by_spindle: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_reports_only_features_implemented_by_the_adapter() {
        for provider in registry() {
            assert!(matches!(provider.streaming, CapabilitySupport::Supported));
            assert!(matches!(
                provider.cancellation,
                CapabilitySupport::Supported
            ));
            assert!(matches!(
                provider.tool_calling,
                CapabilitySupport::ProviderDependent
            ));
            assert!(matches!(
                provider.structured_output,
                CapabilitySupport::NotImplemented
            ));
        }
    }
}
