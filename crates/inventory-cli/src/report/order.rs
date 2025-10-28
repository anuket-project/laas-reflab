/// This is needed because some yaml models need to have their reports
/// executed in a certain order due to foreign key restraints.
/// each model has space for at least 4 variants.
#[repr(u8)]
pub enum SortOrder {
    Lab = 0,
    Flavor = 4,
    Image = 8,
    Switch = 12,
    Switchport = 16,
    Host = 20,
    HostPort = 24,
    KernelArg = 28,
}
