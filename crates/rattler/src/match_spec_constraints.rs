use crate::{MatchSpec, PackageRecord, Range, Version};
use itertools::Itertools;
use once_cell::sync::OnceCell;
use pubgrub::version_set::VersionSet;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};
use std::iter::once;
use std::sync::RwLock;

static COMPLEMENT_CACHE: OnceCell<RwLock<HashMap<MatchSpecConstraints, MatchSpecConstraints>>> =
    OnceCell::new();

/// A single AND group in a `MatchSpecConstraints`
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct MatchSpecElement {
    version: Range<Version>,
    build_number: Range<usize>,
}

impl MatchSpecElement {
    /// Returns an instance that matches nothing.
    fn none() -> Self {
        Self {
            version: Range::none(),
            build_number: Range::none(),
        }
    }

    /// Returns an instance that matches anything.
    fn any() -> Self {
        Self {
            version: Range::any(),
            build_number: Range::any(),
        }
    }

    /// Returns the intersection of this element and another
    fn intersection(&self, other: &Self) -> Self {
        let version = self.version.intersection(&other.version);
        let build_number = self.build_number.intersection(&other.build_number);
        if version == Range::none() || build_number == Range::none() {
            Self::none()
        } else {
            Self {
                version,
                build_number,
            }
        }
    }

    /// Returns true if the specified packages matches this instance
    pub fn contains(&self, package: &PackageRecord) -> bool {
        self.version.contains(&package.version) && self.build_number.contains(&package.build_number)
    }
}

/// Represents several constraints as a DNF.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct MatchSpecConstraints {
    groups: Vec<MatchSpecElement>,
}

impl From<MatchSpec> for MatchSpecConstraints {
    fn from(spec: MatchSpec) -> Self {
        Self {
            groups: vec![MatchSpecElement {
                version: spec.version.map(Into::into).unwrap_or_else(|| Range::any()),
                build_number: spec
                    .build_number
                    .clone()
                    .map(Range::equal)
                    .unwrap_or_else(|| Range::any()),
            }],
        }
    }
}

impl From<MatchSpecElement> for MatchSpecConstraints {
    fn from(elem: MatchSpecElement) -> Self {
        Self { groups: vec![elem] }
    }
}

impl Display for MatchSpecConstraints {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "bla")
    }
}

impl MatchSpecConstraints {
    fn compute_complement(&self) -> Self {
        if self.groups.is_empty() {
            Self {
                groups: vec![MatchSpecElement::any()],
            }
        } else {
            let mut groups: HashSet<_> = [MatchSpecElement::any()].into();
            let mse_none = MatchSpecElement::none();
            for spec in self.groups.iter() {
                let mut next = HashSet::new();
                let version_complement = spec.version.negate();
                if version_complement != Range::none() {
                    let version_complement = MatchSpecElement {
                        version: version_complement,
                        build_number: Range::any(),
                    };
                    next.extend(
                        groups
                            .iter()
                            .map(|o| o.intersection(&version_complement))
                            .filter(|n| n != &mse_none),
                    );
                }

                let build_complement = spec.build_number.negate();
                if build_complement != Range::none() {
                    let build_complement = MatchSpecElement {
                        version: Range::any(),
                        build_number: build_complement,
                    };
                    next.extend(
                        groups
                            .iter()
                            .map(|o| o.intersection(&build_complement))
                            .filter(|n| n != &mse_none),
                    );
                }

                groups = next;
            }

            Self {
                groups: groups
                    .into_iter()
                    .sorted_by_cached_key(|e| {
                        let mut hasher = DefaultHasher::new();
                        e.hash(&mut hasher);
                        hasher.finish()
                    })
                    .collect(),
            }
        }
    }
}

impl VersionSet for MatchSpecConstraints {
    type V = PackageRecord;

    fn empty() -> Self {
        Self { groups: vec![] }
    }

    fn full() -> Self {
        Self {
            groups: vec![MatchSpecElement {
                version: Range::any(),
                build_number: Range::any(),
            }],
        }
    }

    fn singleton(v: Self::V) -> Self {
        Self {
            groups: vec![MatchSpecElement {
                version: Range::equal(v.version),
                build_number: Range::equal(v.build_number),
            }],
        }
    }

    fn complement(&self) -> Self {
        // dbg!("taking the complement of group ",  self.groups.len());

        let complement_cache = COMPLEMENT_CACHE.get_or_init(|| RwLock::new(Default::default()));
        {
            let read_lock = complement_cache.read().unwrap();
            if let Some(result) = read_lock.get(self) {
                return result.clone();
            }
        }

        // dbg!("-- NOT CACHED", self);

        let complement = self.compute_complement();
        {
            let mut write_lock = complement_cache.write().unwrap();
            write_lock.insert(self.clone(), complement.clone());
        }

        return complement;
    }

    fn intersection(&self, other: &Self) -> Self {
        let groups: HashSet<_> = once(self.groups.iter())
            .chain(once(other.groups.iter()))
            .multi_cartesian_product()
            .map(|elems| {
                elems
                    .into_iter()
                    .cloned()
                    .reduce(|a, b| a.intersection(&b))
                    .unwrap()
            })
            .filter(|group| group != &MatchSpecElement::none())
            .collect();

        if groups.iter().any(|group| group == &MatchSpecElement::any()) {
            return MatchSpecElement::any().into();
        }

        let mut groups = groups.into_iter().collect_vec();

        groups.sort_by_cached_key(|e| {
            let mut hasher = DefaultHasher::new();
            e.hash(&mut hasher);
            hasher.finish()
        });

        groups.dedup();

        Self { groups }
    }

    fn contains(&self, v: &Self::V) -> bool {
        self.groups.iter().any(|group| group.contains(v))
    }
}

#[cfg(test)]
mod tests {
    use crate::match_spec_constraints::MatchSpecConstraints;
    use crate::{PackageRecord, Version};
    use pubgrub::version_set::VersionSet;
    use std::str::FromStr;

    #[test]
    fn complement() {
        let record = PackageRecord {
            name: "".to_string(),
            version: Version::from_str("1.2.3").unwrap(),
            build: "".to_string(),
            build_number: 1,
            subdir: "".to_string(),
            md5: None,
            sha256: None,
            arch: None,
            platform: None,
            depends: vec![],
            constrains: vec![],
            track_features: None,
            features: None,
            preferred_env: None,
            license: None,
            license_family: None,
            timestamp: None,
            date: None,
            size: None,
        };

        let constraint = MatchSpecConstraints::singleton(record.clone());

        assert!(constraint.contains(&record));
        assert!(!constraint.complement().contains(&record));

        assert_eq!(constraint.intersection(&constraint), constraint);
        assert_eq!(
            constraint.intersection(&constraint.complement()),
            MatchSpecConstraints::empty()
        );

        assert_eq!(
            constraint
                .complement()
                .complement()
                .complement()
                .complement(),
            constraint
        );
        assert_eq!(
            constraint.complement().complement().complement(),
            constraint.complement()
        );

        assert_eq!(
            MatchSpecConstraints::empty(),
            constraint.complement().intersection(&constraint)
        );
        assert_eq!(
            MatchSpecConstraints::full(),
            constraint.complement().union(&constraint)
        );
    }
}
