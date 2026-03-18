/// Summary statistics for occurrence activation counts.
///
/// Returned by store implementations to provide a snapshot of
/// activation distribution across all persisted occurrences.
pub struct ActivationStats {
    pub total: u64,
    pub zero_activation: u64,
    pub max_activation: u32,
    pub mean_activation: f64,
}
