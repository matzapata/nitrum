# KMS permissions and operations

Every Nitrum cloud deployment creates one symmetric KMS key (alias `alias/{project}-enclave`) through the bundled CloudFormation template. This page explains what that key protects, who is allowed to do what with it, how the PCR0 condition changes when you deploy, and how to debug the failures you are most likely to hit. For the design rationale see [architecture.md](architecture.md#persistent-encryption-key-and-kms).

## What uses the key

The data-plane keeps one data encryption key (DEK) that protects the TLS certificate and other platform state. The DEK is stored in DynamoDB wrapped by the KMS key, and it touches KMS at boot only:

| Boot | KMS calls |
|------|-----------|
| First boot of a new stack (the elected leader) | `GenerateDataKeyWithoutPlaintext` (no attestation), then an attested `Decrypt` of the new DEK |
| Every later boot, on any instance | Attested `Decrypt` of the stored DEK |

"Attested" means the call carries a Nitro `Recipient` built from an attestation document, which is what makes KMS evaluate `kms:RecipientAttestation:ImageSha384`. After the DEK is unwrapped it lives in enclave memory only. Two consequences follow:

- Changing the key policy does not affect enclaves that are already running. Only enclaves that **boot** afterwards are checked against the new PCR0.
- Every boot needs a successful attested `Decrypt`, including the very first one, so enclave restarts, ASG replacements, scale-outs and brand-new stacks are the moments a misconfigured policy shows up.

## Who can do what

Three layers decide whether a call succeeds. For `Decrypt`, **both** the IAM policy and the key policy must allow it.

| Principal | Granted by | Allowed |
|-----------|------------|---------|
| EC2 instance role (what the enclave uses through IMDS) | IAM inline policy `EnclaveKms`, scoped to the key ARN | `kms:GenerateDataKey`, `kms:GenerateDataKeyWithoutPlaintext`, `kms:Decrypt` |
| Account root (delegates to IAM) | Key policy `EnableDecryptFromEnclave` | `kms:Decrypt` **only if** `kms:RecipientAttestation:ImageSha384` equals the deployed EIF's PCR0 |
| Account root (delegates to IAM) | Key policy `EnclaveGenerateDataKey` | `kms:GenerateDataKey`, `kms:GenerateDataKeyWithoutPlaintext`, no condition |
| KMS administrator | Key policy `KmsAdministrator` | `Create*`, `Describe*`, `Enable*`, `List*`, `Put*`, `Update*`, `Revoke*`, `Disable*`, `Get*`, `Delete*`, `ScheduleKeyDeletion`, `CancelKeyDeletion`, `GenerateDataKey`, `TagResource`, `UntagResource` |

Things worth knowing:

- **The administrator cannot decrypt.** `kms:Decrypt` is not in the `KmsAdministrator` statement. Reading the DEK requires an attested enclave whose measurement matches the policy.
- **The administrator is `cloud.kms_administrator_role_arn`**, or account root when it is empty. When you set a role ARN, account root is *not* granted the admin actions, so that role is the only principal that can call `kms:PutKeyPolicy`.
- **Every deploy that ships a new EIF calls `kms:PutKeyPolicy`** (to move the PCR0 condition), so the identity running `nitrum cloud deploy` has to be the administrator or assume that role. `nitrum cloud deploy` warns when `sts:GetCallerIdentity` does not match the configured ARN. Assumed-role sessions of the configured role count as a match.
- Pick a durable role for `kms_administrator_role_arn`. If nobody can act as that principal any more, the PCR0 condition can no longer be updated and new EIFs cannot be deployed.
- To change the administrator itself, deploy as the *current* administrator, since that is also a key policy update.

## How PCR0 follows a deploy

`nitrum cloud deploy` reads PCR0 from the EIF (the same value `nitrum describe` prints) and passes it to CloudFormation as `EifImageSha384`, which rewrites the `EnableDecryptFromEnclave` condition in place. The key id does not change. See [usage.md](usage.md#updating-a-deployment) for the full rolling sequence.

- **During a roll**, the policy already names the new PCR0 while some hosts still run the old EIF. Those hosts keep serving (they hold the DEK in memory), but an old-EIF enclave that restarts before the roll finishes is denied.
- **Rolling back** means deploying the previous EIF again. Reverting the ASG alone does not work, because the policy would still name the newer PCR0.
- **`--debug-mode` deploys pin an all-zero PCR0.** Debug-mode enclaves report zeroed PCRs, so the policy from such a deploy is satisfied by *any* debug-mode enclave that IAM lets call the key. Never use debug mode with real data. Deploying again without `--debug-mode` restores the real PCR0.
- **PCR0 has to be reproducible.** If the same source produces a different EIF hash, the policy and the enclave you run can disagree. See [reproducible builds](usage.md#reproducible-builds).

## Troubleshooting

### The enclave keeps restarting and logs `attested KMS Decrypt denied`

The data-plane logs this once per boot when the attested `Decrypt` returns `AccessDenied` / `AccessDeniedException`, then exits. The event carries:

- `key_id`: the key the data-plane tried to use.
- `condition`: always `kms:RecipientAttestation:ImageSha384`.
- `aws_error_code`: the code KMS returned.
- `pcr0`: the running enclave's PCR0 as read from the NSM, or `(unavailable)` if that read failed (best effort).

Read it with `nitrum cloud logs --service data-plane`, or in the `/nitrum/{project}/data-plane` CloudWatch log group.

**1. Compare the running PCR0 with the policy.**

```bash
# Policy side: look at the EnableDecryptFromEnclave statement
aws kms get-key-policy --key-id alias/{project}-enclave --policy-name default \
  --query Policy --output text

# Image side, if the log said pcr0=(unavailable): PCR0 of the EIF you deployed
# (defaults to the last build in .nitrum/artifacts; pass the EIF path otherwise)
nitrum describe [EIF]
```

**2. Match the difference to a cause.**

| What you see | Likely cause | Fix |
|--------------|--------------|-----|
| Policy PCR0 is newer than the logged `pcr0` | Deploy in progress, or an old-EIF host restarted or was replaced mid-roll | Wait for the roll to finish. If hosts stay on the old EIF, redeploy the EIF you want running. |
| Policy PCR0 is all zeros | The last deploy used `--debug-mode` | Deploy again without `--debug-mode`. |
| Logged `pcr0` is all zeros, policy is not | Debug-mode enclave against a real-PCR0 policy | Deploy again with matching modes. |
| Values differ and no deploy is in flight | Policy edited outside CloudFormation, or a failed update rolled back | Run CloudFormation drift detection, then redeploy. |
| Values are equal | Not a PCR0 problem. The log fires on **any** `AccessDenied` from this call. | Check the next row. |

**3. If PCR0 matches, check IAM and the key policy.**

- The instance role has the `EnclaveKms` statement with `kms:Decrypt` on the stack's key ARN.
- The key policy still contains `EnableDecryptFromEnclave` (nobody removed it).
- `key_id` in the log is the key of this stack: `/nitrum/{project}/data-plane/kms_key_id` in SSM.

**What you will see on the host.** Because the data-plane exits, the control-plane sees the enclave disappear:

- Control-plane log `enclave exited before settle window` with `uptime_ms` and `consecutive_fast_crashes`.
- Restart metric `nitrum.enclave.restarts` with `result=exited_before_stable`.
- Restart delay doubles from 1 s up to 300 s and only resets after an enclave stays up for the 30 s settle window.
- The NLB target never turns healthy, and the `cloud.sns_alarm_topic_arn` alarm fires if you configured it.

If the enclave dies *after* the settle window, the launch counts as stable and backoff resets. In that case you will see repeated `started` restarts instead of `exited_before_stable`; the data-plane log is still the place to look.

### `nitrum cloud deploy` fails with `AccessDenied` on `kms:PutKeyPolicy`

The identity running the deploy is not the KMS administrator. Assume the role in `cloud.kms_administrator_role_arn` (or deploy as account root when the field is empty) and run the deploy again. The CLI prints a warning before the deploy starts when it detects the mismatch.

### Deleting a stack

Without `--retain`, deleting the stack deletes the KMS key **and** the DynamoDB table that holds the wrapped DEK and the stored TLS certificate. A stack re-created under the same name starts from scratch: new key, new DEK, new certificate. Deploy with `--retain` for anything you care about. It retains the key, DynamoDB table, log groups and SSM parameters, and turns on key rotation and DynamoDB point-in-time recovery.
