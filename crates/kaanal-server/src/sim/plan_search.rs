//! The cheapest plan (Step 9c-2, RULES 141 and 142): with `cheapest: true` a plan is the cheapest
//! search (`kaanal_sim::plan::search`) on the design in force instead of the 9c-1 walk, plus one
//! what-if section per other provider: the design translated to that provider's classes
//! (`kaanal_core::resize::translate`) and the same search run on it. The time limit is split by
//! `split_time` (half to the main search when a section runs, the rest equally); each search reads
//! its own monotonic clock between runs. The route checks the request fields here
//! ([`cheapest_asked`]); the job's tail (proposed draft, re-check, sweep, budget) is the 9c-1 one in
//! `control/plan.rs`, on the main search's design only: a section has no draft.

use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use kaanal_core::config::Hosting;
use kaanal_core::resize::{Translation, providers_of, translate};
use kaanal_core::{Config, NodeKind};
use kaanal_sim::Limits;
use kaanal_sim::plan::{
    CHEAPEST_NOTE, Plan, Request, SearchInfo, SearchOptions, Searched, Step, search, split_time,
};
use serde::Serialize;

use super::plan_draft::{BudgetCheck, SATURATED_COST, judge_budget, price_cfg};
use super::run::{Job, price_design};
use kaanal_core::cost::CostReport;
use kaanal_sim::plan::PlanVerdict;

/// The providers a section can be asked for, in the order sections are run and listed.
pub const PROVIDERS: [&str; 3] = ["aws", "gcp", "azure"];
/// `time_limit_s` bounds and default (spec: 2 to 3 minutes is the developer's budget).
const TIME_LIMIT_S: RangeInclusive<u32> = 1..=600;
const DEFAULT_TIME_LIMIT_S: u32 = 150;
/// `run_cap` bounds and default, per search.
const RUN_CAP: RangeInclusive<u32> = 1..=2000;
const DEFAULT_RUN_CAP: u32 = 200;

/// What every provider section says (spec anti-goal: a comparison that hides what it does not model).
pub const PROVIDER_NOTE: &str =
    "assumes wires, egress, data location and application are unchanged; on-demand price estimates";

/// The cheapest fields as checked, on the record (`PlanAsked.cheapest`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheapestAsked {
    /// The whole time limit, split between the searches.
    pub time_limit_s: u32,
    /// The most runs per search.
    pub run_cap: u32,
    /// The providers a section is run for, in [`PROVIDERS`] order (`[]` for `["none"]`).
    pub providers: Vec<String>,
}

/// One provider what-if: the cheapest search on the translated design, or why there is none.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderSection {
    /// The search on the design translated to `provider`.
    Answer {
        /// The provider.
        provider: String,
        /// Its plan (steps, prices as the main plan; no draft). The verdict is judged again against
        /// the budget from `cost`, as the main plan's is from its re-check.
        plan: Box<Plan>,
        /// What its design costs an hour with per-use prices at the visits of one run at the target
        /// (the main plan's `verify.cost` rule).
        cost: CostReport,
        /// [`SATURATED_COST`] when the design does not hold (its per-use prices are understated).
        #[serde(skip_serializing_if = "Option::is_none")]
        cost_note: Option<&'static str>,
        /// The budget against `cost`, when one was asked for.
        #[serde(skip_serializing_if = "Option::is_none")]
        budget_check: Option<BudgetCheck>,
        /// How its search ran.
        info: SearchInfo,
        /// The translation's notes, then [`PROVIDER_NOTE`] and [`CHEAPEST_NOTE`].
        notes: Vec<String>,
    },
    /// No section for `provider`, and why.
    Skipped {
        /// The provider.
        provider: String,
        /// Why, in plain words.
        reason: String,
    },
}

/// The cheapest fields of the body checked against the design in force `cfg`; `None` for a plain
/// plan. Without `cheapest`, any of the other three is refused ("needs cheapest").
///
/// # Errors
/// The 400 message: a field without `cheapest`, a bound crossed, an unknown, repeated or empty
/// provider list, `none` with another name, or a provider the design already uses.
pub(crate) fn cheapest_asked(
    cheapest: bool,
    time_limit_s: Option<u32>,
    run_cap: Option<u32>,
    providers: Option<Vec<String>>,
    cfg: &Config,
) -> Result<Option<CheapestAsked>, String> {
    if !cheapest {
        let set = [
            ("time_limit_s", time_limit_s.is_some()),
            ("run_cap", run_cap.is_some()),
            ("providers", providers.is_some()),
        ];
        return match set.iter().find(|(_, on)| *on) {
            Some((f, _)) => Err(format!("{f} needs cheapest: true")),
            None => Ok(None),
        };
    }
    let time_limit_s = time_limit_s.unwrap_or(DEFAULT_TIME_LIMIT_S);
    if !TIME_LIMIT_S.contains(&time_limit_s) {
        return Err(format!(
            "time_limit_s must be {} to {} seconds",
            TIME_LIMIT_S.start(),
            TIME_LIMIT_S.end()
        ));
    }
    let run_cap = run_cap.unwrap_or(DEFAULT_RUN_CAP);
    if !RUN_CAP.contains(&run_cap) {
        return Err(format!(
            "run_cap must be {} to {}",
            RUN_CAP.start(),
            RUN_CAP.end()
        ));
    }
    let used = providers_of(cfg);
    let providers = match providers {
        // the default: every other provider, when the design has a class to translate
        None if used.is_empty() => Vec::new(),
        None => PROVIDERS
            .iter()
            .filter(|p| !used.iter().any(|u| u == *p))
            .map(|p| (*p).to_string())
            .collect(),
        Some(v) if v == ["none"] => Vec::new(),
        Some(v) => named(&v, &used)?,
    };
    Ok(Some(CheapestAsked {
        time_limit_s,
        run_cap,
        providers,
    }))
}

/// **Private.** A posted provider list, checked, in [`PROVIDERS`] order.
fn named(v: &[String], used: &[String]) -> Result<Vec<String>, String> {
    const WORDS: &str = "providers must be a list of aws, gcp, azure, or [\"none\"]";
    if v.is_empty() {
        return Err(WORDS.into());
    }
    for (i, p) in v.iter().enumerate() {
        if !PROVIDERS.contains(&p.as_str()) {
            return Err(format!("{WORDS}: '{p}' is not one"));
        }
        if v[..i].contains(p) {
            return Err(format!("{WORDS}: '{p}' is named twice"));
        }
        if used.contains(p) {
            return Err(format!(
                "providers: the design already uses {p}; the main plan is that provider's"
            ));
        }
    }
    Ok(PROVIDERS
        .iter()
        .filter(|p| v.iter().any(|x| x == *p))
        .map(|p| (*p).to_string())
        .collect())
}

/// The design translated to each of `providers`, or why not (no run spent). A translation that moves
/// nothing (every class kept as written) is "nothing to translate", and an Azure managed database
/// with no `flavor` is not guessed onto AWS (its engine there is unknown).
pub(crate) fn translations(
    cfg: &Config,
    providers: &[String],
    keep: &[String],
) -> Vec<(String, Result<Translation, String>)> {
    providers
        .iter()
        .map(|to| {
            let t = unknown_flavor(cfg, to, keep)
                .map_or_else(|| translate(cfg, to, keep), Err)
                .and_then(|t| {
                    let moved = cfg.nodes.iter().any(|(k, n)| {
                        t.config.nodes.get(k).map(|m| m.instance_name()) != Some(n.instance_name())
                    });
                    if moved {
                        Ok(t)
                    } else {
                        Err(format!(
                            "nothing to translate: every node is kept as written ({})",
                            t.notes.join("; ")
                        ))
                    }
                });
            (to.clone(), t)
        })
        .collect()
}

/// **Private.** Why a translation to `to` would guess: an Azure managed database (Azure SQL) with
/// no `flavor` written has no known engine on AWS. `translate` would make it PostgreSQL; the
/// section says so instead of pricing a guess.
fn unknown_flavor(cfg: &Config, to: &str, keep: &[String]) -> Option<String> {
    if to != "aws" {
        return None;
    }
    cfg.nodes
        .iter()
        .find(|(name, n)| {
            n.kind() == NodeKind::Database
                && n.hosting() == Hosting::Managed
                && n.flavor().is_none()
                && !keep.contains(name)
                && n.instance_name().is_some_and(|i| i.starts_with("azure/"))
        })
        .map(|(name, _)| {
            format!("{name}: no flavor written, so its engine on aws is unknown; write flavor: to compare")
        })
}

/// What the cheapest job gives the plan's tail: the main search and the sections.
pub(crate) struct Cheapest {
    /// The main search on the design in force.
    pub main: Searched,
    /// One per provider asked, in [`PROVIDERS`] order.
    pub providers: Vec<ProviderSection>,
}

/// The cheapest job (module doc): translations first (pure, no time), then the main search with its
/// share of the limit (each step to `on_step`), then a search per translation with its share and,
/// after it, one pricing run of its design at load factor `factor` (outside the limit, as the main plan's
/// re-check). A section whose search or pricing run fails is skipped with the reason.
///
/// # Errors
/// The main search's: as `kaanal_sim::plan::walk`.
pub(crate) fn run_cheapest(
    job: &Job,
    req: &Request,
    c: &CheapestAsked,
    factor: f64,
    on_step: &mut dyn FnMut(&Step),
) -> Result<Cheapest, String> {
    let tries = translations(&job.g.config, &c.providers, &req.keep);
    let ready = tries.iter().filter(|(_, t)| t.is_ok()).count();
    let (main_t, shares) = split_time(Duration::from_secs(c.time_limit_s.into()), ready);
    let main = searched(&job.g.config, job, req, main_t, c.run_cap, on_step)?;
    let mut shares = shares.into_iter();
    let providers = tries
        .into_iter()
        .map(|(provider, t)| {
            let t = match t {
                Ok(t) => t,
                Err(reason) => return ProviderSection::Skipped { provider, reason },
            };
            let share = shares.next().unwrap_or_default();
            let s = match searched(&t.config, job, req, share, c.run_cap, &mut |_| {}) {
                Ok(s) => s,
                Err(reason) => return ProviderSection::Skipped { provider, reason },
            };
            let price = price_cfg(&job.cfg, factor);
            let cost = match price_design(&s.plan.config, &job.g.endpoints, &job.chaos, &price) {
                Ok(c) => c,
                Err(e) => {
                    let reason = format!("the pricing run of its design: {e}");
                    return ProviderSection::Skipped { provider, reason };
                }
            };
            let mut plan = s.plan;
            let holds = matches!(
                plan.verdict,
                PlanVerdict::AlreadyHolds
                    | PlanVerdict::Holds
                    | PlanVerdict::OverBudget { holds: true, .. }
            );
            let (verdict, budget_check) = judge_budget(
                plan.verdict,
                req.budget_micro_usd_hr,
                cost.total_micro_usd_hr,
            );
            plan.verdict = verdict;
            let mut notes = t.notes;
            notes.push(PROVIDER_NOTE.into());
            notes.push(CHEAPEST_NOTE.into());
            ProviderSection::Answer {
                provider,
                plan: Box::new(plan),
                info: s.info,
                cost,
                cost_note: (!holds).then_some(SATURATED_COST),
                budget_check,
                notes,
            }
        })
        .collect();
    Ok(Cheapest { main, providers })
}

/// **Private.** One search on `cfg` (the design in force or a translation, same node order) with
/// its own clock.
fn searched(
    cfg: &Config,
    job: &Job,
    req: &Request,
    time_limit: Duration,
    run_cap: u32,
    on_step: &mut dyn FnMut(&Step),
) -> Result<Searched, String> {
    let t0 = Instant::now();
    search(
        cfg,
        &job.g.endpoints,
        &job.chaos,
        &job.cfg,
        req,
        &SearchOptions {
            time_limit,
            run_cap,
        },
        &Limits::server(),
        &mut move || t0.elapsed(),
        on_step,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(nodes: &str) -> Config {
        Config::from_yaml(&format!("version: 1\nnodes:\n{nodes}")).unwrap_or_else(|e| panic!("{e}"))
    }

    const AWS: &str = "  s: { type: service, proc: 5ms, instance: aws/m5.large }\n";

    fn asked(p: Option<&[&str]>, c: &Config) -> Result<Option<CheapestAsked>, String> {
        let p = p.map(|v| v.iter().map(|s| (*s).to_string()).collect());
        cheapest_asked(true, None, None, p, c)
    }

    #[test]
    fn the_defaults_and_the_provider_lists() {
        let c = cfg(AWS);
        let a = asked(None, &c).unwrap().unwrap();
        assert_eq!((a.time_limit_s, a.run_cap), (150, 200));
        assert_eq!(
            a.providers,
            ["gcp", "azure"],
            "every other, in a fixed order"
        );
        let a = asked(Some(&["azure", "gcp"]), &c).unwrap().unwrap();
        assert_eq!(
            a.providers,
            ["gcp", "azure"],
            "listed order does not matter"
        );
        assert_eq!(
            asked(Some(&["none"]), &c).unwrap().unwrap().providers,
            Vec::<String>::new()
        );
        // no class to translate: no section by default
        let bare = cfg("  s: { type: service, proc: 5ms }\n");
        assert_eq!(
            asked(None, &bare).unwrap().unwrap().providers,
            Vec::<String>::new()
        );
        assert!(
            asked(Some(&["aws"]), &c)
                .unwrap_err()
                .contains("already uses aws")
        );
        assert_eq!(cheapest_asked(false, None, None, None, &c), Ok(None));
        assert!(
            cheapest_asked(false, Some(1), None, None, &c)
                .unwrap_err()
                .contains("needs cheapest")
        );
    }

    /// Ruling 1: a translation that moves nothing is skipped, not priced as a section.
    #[test]
    fn a_translation_that_moves_nothing_is_skipped() {
        // a cache with no class and a node with no class have none to move
        let c = cfg("  c: { type: cache }\n  e: { type: service, proc: 5ms }\n");
        let t = translations(&c, &["gcp".into()], &[]);
        let why = t[0].1.as_ref().unwrap_err();
        assert!(why.starts_with("nothing to translate"), "{why}");
        assert!(
            why.contains("c: kept as written, not compared (a cache)"),
            "{why}"
        );
        // a cache on a class would stay on aws beside a gcp service: skipped, not priced as a mix
        let c = cfg(
            "  c: { type: cache, hosting: managed, instance: aws/cache.m5.large }\n  e: { type: service, proc: 5ms, instance: aws/m5.large }\n",
        );
        let t = translations(&c, &["gcp".into()], &[]);
        let why = t[0].1.as_ref().unwrap_err();
        assert!(why.contains("would mix providers"), "{why}");
        let c = cfg(AWS);
        assert!(translations(&c, &["gcp".into()], &[])[0].1.is_ok());
        // keeping a node with a class would mix providers: refused by the translation (defect 2)
        assert_eq!(
            translations(&c, &["gcp".into()], &["s".into()])[0]
                .1
                .as_ref()
                .unwrap_err(),
            "s: kept on aws, so a section on gcp would mix providers"
        );
    }

    /// Ruling 2: an Azure SQL database with no flavor is not guessed onto AWS as PostgreSQL; with a
    /// flavor written it translates.
    #[test]
    fn an_azure_sql_database_with_no_flavor_is_not_guessed_onto_aws() {
        let c = cfg("  d: { type: database, hosting: managed, instance: azure/db.gp_gen5_4 }\n");
        let t = translations(&c, &["aws".into()], &[]);
        assert_eq!(
            t[0].1.as_ref().unwrap_err(),
            "d: no flavor written, so its engine on aws is unknown; write flavor: to compare"
        );
        // kept: not translated, so nothing to guess; it would stay on Azure, a mixed design
        let kept = translations(&c, &["aws".into()], &["d".into()]);
        assert_eq!(
            kept[0].1.as_ref().unwrap_err(),
            "d: kept on azure, so a section on aws would mix providers"
        );
        let c = cfg(
            "  d: { type: database, flavor: sqlserver, hosting: managed, instance: azure/db.gp_gen5_4 }\n",
        );
        let t = translations(&c, &["aws".into()], &[]);
        assert!(t[0].1.is_ok(), "{:?}", t[0].1.as_ref().err());
    }
}
