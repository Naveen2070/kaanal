// The cloud instance table of `instance:` (step 8P), the same names the server knows
// (`crates/kaanal-core/src/instances.rs`; a test there keeps the two lists equal). Estimates from
// the providers' public pages, as of 2026-11: a vCPU counts as a full core here, and the network
// figure is the peak rate, so a preset is an upper bound on capacity, not on speed.

export interface InstanceType {
  name: string;
  vcpu: number;
  /** Memory in GiB. */
  gib: number;
  /** Peak network rate, Gbit/s. */
  gbps: number;
}

const t = (name: string, vcpu: number, gib: number, gbps: number): InstanceType => ({
  name,
  vcpu,
  gib,
  gbps,
});

export const INSTANCE_TYPES: readonly InstanceType[] = [
  t("aws/c6i.2xlarge", 8, 16, 12.5),
  t("aws/c6i.4xlarge", 16, 32, 12.5),
  t("aws/c6i.large", 2, 4, 12.5),
  t("aws/c6i.xlarge", 4, 8, 12.5),
  t("aws/m5.2xlarge", 8, 32, 10),
  t("aws/m5.4xlarge", 16, 64, 10),
  t("aws/m5.large", 2, 8, 10),
  t("aws/m5.xlarge", 4, 16, 10),
  t("aws/r6i.2xlarge", 8, 64, 12.5),
  t("aws/r6i.large", 2, 16, 12.5),
  t("aws/r6i.xlarge", 4, 32, 12.5),
  t("aws/t3.large", 2, 8, 5),
  t("aws/t3.medium", 2, 4, 5),
  t("aws/t3.micro", 2, 1, 5),
  t("aws/t3.small", 2, 2, 5),
  t("azure/standard_d2s_v5", 2, 8, 12.5),
  t("azure/standard_d4s_v5", 4, 16, 12.5),
  t("azure/standard_d8s_v5", 8, 32, 12.5),
  t("gcp/e2-medium", 2, 4, 2),
  t("gcp/e2-standard-2", 2, 8, 4),
  t("gcp/e2-standard-4", 4, 16, 8),
  t("gcp/n2-standard-2", 2, 8, 10),
  t("gcp/n2-standard-4", 4, 16, 10),
  t("gcp/n2-standard-8", 8, 32, 16),
];

export const instanceByName = (name: string): InstanceType | undefined =>
  INSTANCE_TYPES.find((i) => i.name === name.trim().toLowerCase());

/** What an instance fills, in one line: `2 vCPU, 8 GiB, up to 10 Gbit/s`. */
export const instanceLine = (i: InstanceType): string =>
  `${i.vcpu} vCPU, ${i.gib} GiB, up to ${i.gbps} Gbit/s`;
