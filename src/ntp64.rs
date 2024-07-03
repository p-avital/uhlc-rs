//
// Copyright (c) 2017, 2020 ADLINK Technology Inc.
//
// This program and the accompanying materials are made available under the
// terms of the Eclipse Public License 2.0 which is available at
// http://www.eclipse.org/legal/epl-2.0, or the Apache License, Version 2.0
// which is available at https://www.apache.org/licenses/LICENSE-2.0.
//
// SPDX-License-Identifier: EPL-2.0 OR Apache-2.0
//
use alloc::string::String;
use core::fmt;
use core::ops::{Add, AddAssign, Sub, SubAssign};
use core::time::Duration;
use serde::{Deserialize, Serialize};

#[cfg(feature = "std")]
use {
    core::str::FromStr,
    humantime::format_rfc3339_nanos,
    std::time::{SystemTime, UNIX_EPOCH},
};

// maximal number of seconds that can be represented in the 32-bits part
const MAX_NB_SEC: u64 = (1u64 << 32) - 1;
// number of NTP fraction per second (2^32)
const FRAC_PER_SEC: u64 = 1u64 << 32;
// Bit-mask for the fraction of a second part within an NTP timestamp
const FRAC_MASK: u64 = 0xFFFF_FFFFu64;

// number of nanoseconds in 1 second
const NANO_PER_SEC: u64 = 1_000_000_000;

/// A NTP 64-bits format as specified in
/// [RFC-5909](https://tools.ietf.org/html/rfc5905#section-6)
///
/// ```text
/// 0                   1                   2                   3
/// 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |                            Seconds                            |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |                            Fraction                           |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// ```
///
/// The 1st 32-bits part is the number of second since the EPOCH of the physical clock,
/// and the 2nd 32-bits part is the fraction of second.  
/// In case it's part of a [`crate::Timestamp`] generated by an [`crate::HLC`] the last few bits
/// of the Fraction part are replaced by the HLC logical counter.
/// The size of this counter is currently hard-coded as [`crate::CSIZE`].
///
/// ## Conversion to/from String
/// 2 different String representations are supported:
/// 1. **as an unsigned integer in decimal format**
///   - Such conversion is lossless and thus bijective.
///   - NTP64 to String: use [`std::fmt::Display::fmt()`] or [`std::string::ToString::to_string()`].
///   - String to NTP64: use [`std::str::FromStr::from_str()`]
/// 2. **as a [RFC3339](https://www.rfc-editor.org/rfc/rfc3339.html#section-5.8) (human readable) format**:
///   - Such conversion loses some precision because of rounding when conferting the fraction part to nanoseconds
///   - As a consequence it's not bijective: a NTP64 converted to RFC3339 String and then converted back to NTP64 might result to a different time.
///   - NTP64 to String: use [`std::fmt::Display::fmt()`] with the alternate flag (`{:#}`) or [`NTP64::to_string_rfc3339_lossy()`].
///   - String to NTP64: use [`NTP64::parse_rfc3339()`]
///
/// ## On EPOCH
/// This timestamp in actually similar to a [`std::time::Duration`], as it doesn't define an EPOCH.  
/// Only [`NTP64::to_system_time()`], [`NTP64::to_string_rfc3339_lossy()`] and [`std::fmt::Display::fmt()`] (when using `{:#}` alternate flag)
/// operations assume that it's relative to UNIX_EPOCH (1st Jan 1970) to display the timestamp in RFC-3339 format.
#[derive(Copy, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct NTP64(pub u64);

impl NTP64 {
    /// Returns this NTP64 as a u64.
    #[inline]
    pub fn as_u64(&self) -> u64 {
        self.0
    }

    /// Returns this NTP64 as a f64 in seconds.
    ///
    /// The integer part of the f64 is the NTP64's Seconds part.  
    /// The decimal part of the f64 is the result of a division of NTP64's Fraction part divided by 2^32.  
    /// Considering the probable large number of Seconds (for a time relative to UNIX_EPOCH), the precision of the resulting f64 might be in the order of microseconds.
    /// Therefore, it should not be used for comparison. Directly comparing [NTP64] objects is preferable.
    #[inline]
    pub fn as_secs_f64(&self) -> f64 {
        let secs: f64 = self.as_secs() as f64;
        let subsec: f64 = ((self.0 & FRAC_MASK) as f64) / FRAC_PER_SEC as f64;
        secs + subsec
    }

    /// Returns the 32-bits seconds part.
    #[inline]
    pub fn as_secs(&self) -> u32 {
        (self.0 >> 32) as u32
    }

    /// Returns the 32-bits fraction of second part converted to nanoseconds.
    #[inline]
    pub fn subsec_nanos(&self) -> u32 {
        let frac = self.0 & FRAC_MASK;
        ((frac * NANO_PER_SEC) / FRAC_PER_SEC) as u32
    }

    /// Convert to a [`Duration`].
    #[inline]
    pub fn to_duration(self) -> Duration {
        Duration::new(self.as_secs().into(), self.subsec_nanos())
    }

    /// Convert to a [`SystemTime`] (making the assumption that this NTP64 is relative to [`UNIX_EPOCH`]).
    #[inline]
    #[cfg(feature = "std")]
    pub fn to_system_time(self) -> SystemTime {
        UNIX_EPOCH + self.to_duration()
    }

    /// Convert to a RFC3339 time representation with nanoseconds precision.
    /// e.g.: `"2024-07-01T13:51:12.129693000Z"``
    #[cfg(feature = "std")]
    pub fn to_string_rfc3339_lossy(&self) -> String {
        format_rfc3339_nanos(self.to_system_time()).to_string()
    }

    /// Parse a RFC3339 time representation into a NTP64.
    #[cfg(feature = "std")]
    pub fn parse_rfc3339(s: &str) -> Result<Self, ParseNTP64Error> {
        match humantime::parse_rfc3339(s) {
            Ok(time) => time
                .duration_since(UNIX_EPOCH)
                .map(NTP64::from)
                .map_err(|e| ParseNTP64Error {
                    cause: format!("Failed to parse '{s}' : {e}"),
                }),
            Err(_) => Err(ParseNTP64Error {
                cause: format!("Failed to parse '{s}' : invalid RFC3339 format"),
            }),
        }
    }
}

impl Add for NTP64 {
    type Output = Self;

    #[inline]
    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl<'a> Add<NTP64> for &'a NTP64 {
    type Output = <NTP64 as Add<NTP64>>::Output;

    #[inline]
    fn add(self, other: NTP64) -> <NTP64 as Add<NTP64>>::Output {
        Add::add(*self, other)
    }
}

impl Add<&NTP64> for NTP64 {
    type Output = <NTP64 as Add<NTP64>>::Output;

    #[inline]
    fn add(self, other: &NTP64) -> <NTP64 as Add<NTP64>>::Output {
        Add::add(self, *other)
    }
}

impl Add<&NTP64> for &NTP64 {
    type Output = <NTP64 as Add<NTP64>>::Output;

    #[inline]
    fn add(self, other: &NTP64) -> <NTP64 as Add<NTP64>>::Output {
        Add::add(*self, *other)
    }
}

impl Add<u64> for NTP64 {
    type Output = Self;

    #[inline]
    fn add(self, other: u64) -> Self {
        Self(self.0 + other)
    }
}

impl AddAssign<u64> for NTP64 {
    #[inline]
    fn add_assign(&mut self, other: u64) {
        *self = Self(self.0 + other);
    }
}

impl Sub for NTP64 {
    type Output = Self;

    #[inline]
    fn sub(self, other: Self) -> Self {
        Self(self.0 - other.0)
    }
}

impl<'a> Sub<NTP64> for &'a NTP64 {
    type Output = <NTP64 as Sub<NTP64>>::Output;

    #[inline]
    fn sub(self, other: NTP64) -> <NTP64 as Sub<NTP64>>::Output {
        Sub::sub(*self, other)
    }
}

impl Sub<&NTP64> for NTP64 {
    type Output = <NTP64 as Sub<NTP64>>::Output;

    #[inline]
    fn sub(self, other: &NTP64) -> <NTP64 as Sub<NTP64>>::Output {
        Sub::sub(self, *other)
    }
}

impl Sub<&NTP64> for &NTP64 {
    type Output = <NTP64 as Sub<NTP64>>::Output;

    #[inline]
    fn sub(self, other: &NTP64) -> <NTP64 as Sub<NTP64>>::Output {
        Sub::sub(*self, *other)
    }
}

impl Sub<u64> for NTP64 {
    type Output = Self;

    #[inline]
    fn sub(self, other: u64) -> Self {
        Self(self.0 - other)
    }
}

impl SubAssign<u64> for NTP64 {
    #[inline]
    fn sub_assign(&mut self, other: u64) {
        *self = Self(self.0 - other);
    }
}

impl fmt::Display for NTP64 {
    /// By default formats the value as an unsigned integer in decimal format.  
    /// If the alternate flag `{:#}` is used, formats the value with RFC3339 representation with nanoseconds precision.
    ///
    /// # Examples
    /// ```
    ///   use uhlc::NTP64;
    ///
    ///   let t = NTP64(7386690599959157260);
    ///   println!("{t}");    // displays: 7386690599959157260
    ///   println!("{t:#}");  // displays: 2024-07-01T15:32:06.860479000Z
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // if "{:#}" flag is specified, use RFC3339 representation
        if f.alternate() {
            #[cfg(feature = "std")]
            return write!(f, "{}", format_rfc3339_nanos(self.to_system_time()));
            #[cfg(not(feature = "std"))]
            return write!(f, "{}", self.0);
        } else {
            write!(f, "{}", self.0)
        }
    }
}

impl fmt::Debug for NTP64 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Duration> for NTP64 {
    fn from(duration: Duration) -> NTP64 {
        let secs = duration.as_secs();
        assert!(secs <= MAX_NB_SEC);
        let nanos: u64 = duration.subsec_nanos().into();
        NTP64((secs << 32) + ((nanos * FRAC_PER_SEC) / NANO_PER_SEC) + 1)
    }
}

#[cfg(feature = "std")]
impl FromStr for NTP64 {
    type Err = ParseNTP64Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        u64::from_str(s).map(NTP64).map_err(|_| ParseNTP64Error {
            cause: format!("Invalid NTP64 time : '{s}' (must be a u64)"),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ParseNTP64Error {
    pub cause: String,
}

mod tests {

    #[test]
    fn as_secs_f64() {
        use crate::*;

        let epoch = NTP64::default();
        assert_eq!(epoch.as_secs_f64(), 0f64);

        let epoch_plus_1 = NTP64(1);
        assert!(epoch_plus_1 > epoch);
        assert!(epoch_plus_1.as_secs_f64() > epoch.as_secs_f64());

        // test that Timestamp precision is less than announced (3.5ns) in README.md
        let epoch_plus_counter_max = NTP64(CMASK);
        println!(
            "Time precision = {} ns",
            epoch_plus_counter_max.as_secs_f64() * (ntp64::NANO_PER_SEC as f64)
        );
        assert!(epoch_plus_counter_max.as_secs_f64() < 0.0000000035f64);
    }

    #[test]
    fn bijective_to_string() {
        use crate::*;
        use rand::prelude::*;
        use std::str::FromStr;

        let mut rng = rand::thread_rng();
        for _ in 0u64..10000 {
            let t = NTP64(rng.gen());
            assert_eq!(t, NTP64::from_str(&t.to_string()).unwrap());
        }
    }

    #[test]
    fn rfc3339_conversion() {
        use crate::*;
        use regex::Regex;

        let rfc3339_regex = Regex::new(
            r"^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]Z$"
        ).unwrap();

        let now = SystemTime::now();
        let t = NTP64::from(now.duration_since(UNIX_EPOCH).unwrap());

        let rfc3339 = t.to_string_rfc3339_lossy();
        assert_eq!(rfc3339, humantime::format_rfc3339_nanos(now).to_string());
        assert!(rfc3339_regex.is_match(&rfc3339));

        // Test that alternate format "{:#}" displays in RFC3339 format
        let rfc3339_2 = format!("{t:#}");
        assert_eq!(rfc3339_2, humantime::format_rfc3339_nanos(now).to_string());
        assert!(rfc3339_regex.is_match(&rfc3339_2));
    }
}
