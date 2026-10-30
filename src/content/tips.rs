use super::{Category, Tip};

/// Curated, actionable tradecraft and career advice. Ids are stable and are
/// used by the bot for deep links, so never renumber or recycle one.
pub static TIPS: &[Tip] = &[
    Tip {
        id: "ad-tier-zero",
        text: "Map your tier-zero assets (domain controllers, PKI, backup infrastructure) and restrict interactive logons to them. Most domain-wide compromises start with privileged credentials landing on a lower-tier host.",
        category: Category::Blue,
    },
    Tip {
        id: "alert-tuning-baseline",
        text: "Baseline normal behaviour for a week before writing detections. Rules authored without a baseline mostly detect your own administrators.",
        category: Category::Blue,
    },
    Tip {
        id: "api-authz-tests",
        text: "Test object-level authorisation on every endpoint by replaying requests with a second account's identifiers. Broken object-level authorisation remains the most common API flaw.",
        category: Category::Web,
    },
    Tip {
        id: "aslr-is-not-a-defence",
        text: "Treat ASLR as a speed bump, not a control: hunt for information leaks, partial overwrites and non-PIE binaries. Assume the address can be recovered before you assume exploitation is impossible.",
        category: Category::Pwn,
    },
    Tip {
        id: "audit-sudoers",
        text: "Review sudoers for wildcards and NOPASSWD entries that allow argument injection. `sudo vim`, `sudo less` or `sudo tar` are root without any exploit.",
        category: Category::Blue,
    },
    Tip {
        id: "backup-offline-copy",
        text: "Keep at least one backup copy offline or immutable, and restore something real every quarter. Ransomware crews target backup infrastructure first because it removes your recovery options.",
        category: Category::Blue,
    },
    Tip {
        id: "beacon-jitter-analysis",
        text: "Hunt command-and-control by looking for periodic outbound connections with jitter, not by domain reputation alone. Beacon timing survives domain rotation.",
        category: Category::Blue,
    },
    Tip {
        id: "burp-scope-discipline",
        text: "Configure Burp scope before the first request and keep out-of-scope hosts excluded at the proxy. Testing third-party infrastructure by accident is a legal incident, not a finding.",
        category: Category::Web,
    },
    Tip {
        id: "c2-egress-filtering",
        text: "Deny outbound traffic by default and log DNS queries. Most command-and-control still needs an egress path you could have closed.",
        category: Category::Network,
    },
    Tip {
        id: "cloud-imds-v2",
        text: "Enforce IMDSv2 and restrict metadata access on every instance. SSRF to the instance metadata endpoint is still a standard route to cloud credentials.",
        category: Category::Cloud,
    },
    Tip {
        id: "cloud-root-account",
        text: "Remove access keys from the cloud root account, put hardware MFA on it, and use it only for the few tasks that genuinely require it.",
        category: Category::Cloud,
    },
    Tip {
        id: "cloud-trail-all-regions",
        text: "Enable control-plane logging in every region and ship it to a separate account the workload cannot delete. Attackers disable logging in the region they use, not the one you watch.",
        category: Category::Cloud,
    },
    Tip {
        id: "container-no-root",
        text: "Run containers as non-root with a read-only root filesystem and dropped capabilities. It removes most post-exploitation options inside the container.",
        category: Category::Cloud,
    },
    Tip {
        id: "crypto-never-roll-your-own",
        text: "Never invent a cipher, mode or padding scheme. Use a vetted library, an AEAD construction and a fresh random nonce per message.",
        category: Category::Crypto,
    },
    Tip {
        id: "ctf-writeup-habit",
        text: "Write a short writeup after every challenge, including the ones you failed. Explaining the dead ends is what converts practice into skill.",
        category: Category::Misc,
    },
    Tip {
        id: "cve-triage-exploitability",
        text: "Triage CVEs by exposure and reachability, not by CVSS alone. A 9.8 on an unreachable internal service is not your emergency.",
        category: Category::Blue,
    },
    Tip {
        id: "detection-behaviour-naming",
        text: "Name detections after behaviour, such as LSASS handle access, not after a tool. Tool names change every season; behaviour persists.",
        category: Category::Blue,
    },
    Tip {
        id: "dfir-preserve-volatile-order",
        text: "Collect volatile data before powering a host down: memory, network state and running processes, then disk. Shutting down first destroys the evidence you cannot recover.",
        category: Category::Forensics,
    },
    Tip {
        id: "dfir-timeline-first",
        text: "Build a super-timeline early and let it drive the investigation. It exposes gaps in your evidence collection faster than any single artifact.",
        category: Category::Forensics,
    },
    Tip {
        id: "dns-tunnelling-detection",
        text: "Detect DNS tunnelling using query length, label entropy and volume per parent domain rather than a blocklist of known tools.",
        category: Category::Blue,
    },
    Tip {
        id: "docker-socket-danger",
        text: "Never mount the Docker socket into a container you do not fully trust. Access to it is effectively root on the host.",
        category: Category::Cloud,
    },
    Tip {
        id: "egress-proxy-for-tooling",
        text: "Route offensive tooling and research traffic through infrastructure you control, and keep personal browsing off it. Correlation is cheap for defenders.",
        category: Category::Misc,
    },
    Tip {
        id: "email-auth-hardening",
        text: "Publish SPF, DKIM and DMARC with a reject policy, then monitor the reports. It removes a large share of invoice and brand impersonation before it reaches a mailbox.",
        category: Category::Blue,
    },
    Tip {
        id: "enum-before-exploit",
        text: "Spend more time enumerating than exploiting. Most footholds come from a forgotten service or a leaked credential, not from a zero-day.",
        category: Category::Misc,
    },
    Tip {
        id: "exploit-offset-hygiene",
        text: "When an exploit crashes unpredictably, verify offsets against the exact libc and binary build first. Version mismatch explains most bugs that look unexploitable.",
        category: Category::Pwn,
    },
    Tip {
        id: "fido2-for-admins",
        text: "Require phishing-resistant MFA for administrators and helpdesk staff before anyone else. Those accounts can reset everybody else's credentials.",
        category: Category::Misc,
    },
    Tip {
        id: "firmware-inventory",
        text: "Inventory BMC, hypervisor and edge-appliance firmware versions. Out-of-band management interfaces are rarely patched and frequently internet-facing.",
        category: Category::Blue,
    },
    Tip {
        id: "ghidra-type-recovery",
        text: "Apply struct types and rename variables as you reverse. A decompiler listing only becomes readable after type recovery, not before.",
        category: Category::Reverse,
    },
    Tip {
        id: "git-history-secrets",
        text: "Scan git history, not just the working tree, for committed secrets. Rotating the exposed key matters far more than deleting the file.",
        category: Category::Web,
    },
    Tip {
        id: "golden-ticket-detection",
        text: "Detect forged Kerberos tickets by validating TGT lifetimes and encryption types against domain policy. Forged tickets often carry implausible durations.",
        category: Category::Blue,
    },
    Tip {
        id: "hashcat-mode-check",
        text: "Confirm the exact hash format with `hashid` or `hashcat --example-hashes` before starting a long run. The wrong mode wastes hours and produces nothing.",
        category: Category::Crypto,
    },
    Tip {
        id: "honeytoken-accounts",
        text: "Create decoy service accounts with no legitimate use and alert on any authentication attempt against them. Real users never touch them, so any hit is worth investigating.",
        category: Category::Blue,
    },
    Tip {
        id: "iam-least-privilege-audit",
        text: "Generate a policy from observed control-plane usage and remove what is unused. Wildcard permissions accumulate and never shrink on their own.",
        category: Category::Cloud,
    },
    Tip {
        id: "incident-comms-template",
        text: "Prepare an incident communication template with roles, update cadence and escalation contacts before you need it. Writing it mid-incident costs the first hour you cannot spare.",
        category: Category::Blue,
    },
    Tip {
        id: "ioc-shelf-life",
        text: "Treat IP and domain indicators as short-lived and behaviour as durable. Re-hunting last quarter's infrastructure mostly wastes analyst time.",
        category: Category::Blue,
    },
    Tip {
        id: "jwt-alg-confusion",
        text: "Test JWT handling for `alg: none`, algorithm confusion and missing signature verification before attacking application logic. Libraries default to trusting the header too often.",
        category: Category::Web,
    },
    Tip {
        id: "kdf-parameters-matter",
        text: "Tune password KDF parameters so a single hash costs real time and memory. Argon2id with sane memory settings beats an iterated hash that a GPU eats for breakfast.",
        category: Category::Crypto,
    },
    Tip {
        id: "kerberoasting-detection",
        text: "Alert on bulk service-ticket requests, especially with RC4 encryption, from a single principal. Legitimate applications rarely request dozens of service tickets in a minute.",
        category: Category::Blue,
    },
    Tip {
        id: "kubernetes-rbac-review",
        text: "Review cluster role bindings for `cluster-admin` and wildcard verbs, then remove the ones nobody can justify. Default manifests grant far more than workloads need.",
        category: Category::Cloud,
    },
    Tip {
        id: "lab-snapshot-first",
        text: "Snapshot your lab VM before every exercise. Being able to reset in seconds is what makes you willing to break things and learn from it.",
        category: Category::Misc,
    },
    Tip {
        id: "ldap-signing-required",
        text: "Enforce LDAP signing and channel binding on domain controllers to blunt relay attacks. Without it, any captured authentication can be forwarded.",
        category: Category::Network,
    },
    Tip {
        id: "linux-auditd-rules",
        text: "Log `execve`, privilege changes and sensitive file access with auditd, then verify the rules actually fire. An unvalidated audit rule is only documentation.",
        category: Category::Blue,
    },
    Tip {
        id: "logging-centralised",
        text: "Ship logs off the host to a system attackers cannot easily delete. Local-only logs are the first thing wiped after a foothold.",
        category: Category::Blue,
    },
    Tip {
        id: "macro-block-policy",
        text: "Block Office macros originating from the internet and alert on attempts to enable them. It breaks a whole family of commodity loaders at almost no cost.",
        category: Category::Blue,
    },
    Tip {
        id: "malware-detonation-isolation",
        text: "Detonate samples in an isolated VM with host-only networking and a snapshot. Never open an unknown sample on the workstation you use for analysis.",
        category: Category::Forensics,
    },
    Tip {
        id: "memory-safety-hardening",
        text: "When you must ship C or C++, enable stack protection, `_FORTIFY_SOURCE`, RELRO and PIE, then confirm the flags in the built binary. Build systems silently drop them.",
        category: Category::Pwn,
    },
    Tip {
        id: "mitre-attack-mapping",
        text: "Map each detection to an ATT&CK technique and record the gaps. Coverage conversations become concrete instead of anecdotal.",
        category: Category::Blue,
    },
    Tip {
        id: "mobile-cert-pinning",
        text: "Check for certificate pinning before proxying a mobile app. Bypassing it is often the real first step of the assessment.",
        category: Category::Web,
    },
    Tip {
        id: "netflow-baseline",
        text: "Retain NetFlow or equivalent metadata for at least 30 days. Retro-hunting is impossible without historical flow data.",
        category: Category::Network,
    },
    Tip {
        id: "nmap-timing-discipline",
        text: "Use aggressive timing sparingly and prefer targeted scans over full-range sweeps on production ranges. Loud scans trip IDS and wake up the on-call engineer.",
        category: Category::Network,
    },
    Tip {
        id: "nonce-reuse-fatal",
        text: "Treat nonce reuse in stream ciphers and GCM as a catastrophic bug, not a style issue. One repeat leaks plaintext relationships and can expose the authentication key.",
        category: Category::Crypto,
    },
    Tip {
        id: "osint-no-interaction",
        text: "Prefer passive collection: touching a target's systems creates logs and can tip them off. Only interact when the engagement explicitly allows it.",
        category: Category::Osint,
    },
    Tip {
        id: "osint-provenance",
        text: "Record URL, timestamp and a capture for every OSINT claim. Without provenance your finding is a rumour that cannot survive review.",
        category: Category::Osint,
    },
    Tip {
        id: "osint-sock-puppet-hygiene",
        text: "Keep research personas fully separate: dedicated browser profile, email and network path. Cross-contaminated personas are the fastest way to burn an investigation.",
        category: Category::Osint,
    },
    Tip {
        id: "owasp-asvs-baseline",
        text: "Use OWASP ASVS levels as an acceptance checklist for new applications instead of relying on a single pre-launch pentest.",
        category: Category::Web,
    },
    Tip {
        id: "pcap-object-carving",
        text: "Export objects from a pcap before reading payloads by hand. Carving files and streams out of the capture saves hours of manual reassembly.",
        category: Category::Forensics,
    },
    Tip {
        id: "phishing-report-button",
        text: "Give users a one-click report button and close the loop with them on what happened. Reporting rate predicts resilience better than click rate.",
        category: Category::Blue,
    },
    Tip {
        id: "pivot-tooling-mastery",
        text: "Learn one tunnelling approach well, whether SSH, chisel or ligolo, and document the setup. Improvising a pivot under engagement pressure is where people get stuck.",
        category: Category::Network,
    },
    Tip {
        id: "powershell-scriptblock-logging",
        text: "Enable PowerShell script block logging and transcription, then alert on encoded commands and download cradles. Plain command-line logging alone misses most of the content.",
        category: Category::Blue,
    },
    Tip {
        id: "privesc-enumeration-comprehension",
        text: "Understand every finding your privilege-escalation script reports before exploiting it. Blindly running the exploit makes you noisy and gets you stuck.",
        category: Category::Misc,
    },
    Tip {
        id: "purple-team-exercise",
        text: "Run a purple-team session where each technique is executed and the matching detection is verified in the same window. It turns two teams' backlogs into one prioritised list.",
        category: Category::Blue,
    },
    Tip {
        id: "ransomware-canary-files",
        text: "Place canary files in common share locations and alert on modification. It buys minutes while encryption is still spreading.",
        category: Category::Blue,
    },
    Tip {
        id: "regex-redos-review",
        text: "Review user-supplied regexes and email validators for catastrophic backtracking. ReDoS takes down single-threaded services with one well-crafted string.",
        category: Category::Web,
    },
    Tip {
        id: "reverse-dynamic-first",
        text: "Start dynamic: run the sample and watch file, registry and network behaviour, then reverse only the code paths that matter. Static reversing everything first is slower.",
        category: Category::Reverse,
    },
    Tip {
        id: "sast-in-ci",
        text: "Run SAST and dependency scanning on every pull request and fail the build on new high-severity findings. Periodic scans produce reports nobody reads.",
        category: Category::Web,
    },
    Tip {
        id: "secrets-manager-runtime",
        text: "Load secrets from a managed secret store at runtime instead of baking them into environment variables or images. Rotation becomes possible; leaks become recoverable.",
        category: Category::Cloud,
    },
    Tip {
        id: "social-engineering-callback",
        text: "Verify unusual requests through a second channel you already trust, never through contact details supplied in the request itself.",
        category: Category::Misc,
    },
    Tip {
        id: "sqlmap-parameter-scope",
        text: "Point sqlmap at a single parameter with `--batch` and a rate limit. Indiscriminate runs damage data and destroy the client relationship.",
        category: Category::Web,
    },
    Tip {
        id: "ssh-key-hygiene",
        text: "Use per-host keys or short-lived certificates, disable password authentication and remove keys when someone leaves. Shared keys survive long after the person does.",
        category: Category::Network,
    },
    Tip {
        id: "supply-chain-lockfiles",
        text: "Commit lockfiles and pin CI dependencies to versions or digests. An unpinned build is a dependency nobody owns.",
        category: Category::Web,
    },
    Tip {
        id: "sysmon-tuned-config",
        text: "Deploy a tuned Sysmon configuration instead of defaults, then check that process lineage is actually captured. Default logging misses the parent-child relationships you need.",
        category: Category::Blue,
    },
    Tip {
        id: "threat-model-first",
        text: "Threat-model before choosing controls: name the asset, the adversary and the path. That beats buying another tool that addresses none of them.",
        category: Category::Misc,
    },
    Tip {
        id: "triage-playbook-phishing",
        text: "Keep a phishing triage playbook with header analysis, URL detonation steps and mailbox purge commands ready. Improvising this at 2am is how evidence gets lost.",
        category: Category::Blue,
    },
    Tip {
        id: "tryhackme-vs-htb-balance",
        text: "Alternate guided rooms with unguided boxes. Guided content teaches mechanics; unguided boxes teach you to think when nothing tells you what to do next.",
        category: Category::Misc,
    },
    Tip {
        id: "version-pinning-prod",
        text: "Pin production versions and upgrade deliberately. Floating `latest` tags make rollbacks and post-incident reconstruction guesswork.",
        category: Category::Misc,
    },
    Tip {
        id: "vuln-disclosure-scope",
        text: "Read scope and safe-harbour terms before testing a bug bounty target. Out-of-scope testing can end a career, not just an engagement.",
        category: Category::Misc,
    },
    Tip {
        id: "waf-is-not-a-fix",
        text: "Fix the vulnerability, not the payload. WAF rules are a delay tactic and are usually bypassed within days.",
        category: Category::Web,
    },
    Tip {
        id: "windows-laps",
        text: "Use LAPS or an equivalent managed local-admin password solution. Identical local administrator passwords across hosts enable instant lateral movement.",
        category: Category::Blue,
    },
    Tip {
        id: "xss-csp-depth",
        text: "Deploy a strict CSP as defence in depth, but keep context-aware output encoding as the primary control. Permissive CSPs that allow inline script are common.",
        category: Category::Web,
    },
    Tip {
        id: "yara-rule-testing",
        text: "Test YARA rules against a benign corpus before deploying. A rule that fires on everything gets disabled, which is worse than no rule.",
        category: Category::Forensics,
    },
];
