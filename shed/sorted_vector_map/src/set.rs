/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is dual-licensed under either the MIT license found in the
 * LICENSE-MIT file in the root directory of this source tree or the Apache
 * License, Version 2.0 found in the LICENSE-APACHE file in the root directory
 * of this source tree. You may select, at your option, one of the
 * above-listed licenses.
 */

//! Ordered set implementation using a sorted vector

use std::borrow::Borrow;
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::collections::Bound;
use std::collections::Bound::*;
use std::fmt;
use std::fmt::Debug;
use std::iter::Peekable;
use std::mem;
use std::ops::BitAnd;
use std::ops::BitOr;
use std::ops::BitXor;
use std::ops::RangeBounds;
use std::ops::Sub;

use itertools::Itertools;
use quickcheck::Arbitrary;
use quickcheck::Gen;

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
pub struct SortedVectorSet<T>(Vec<T>);

impl<T> SortedVectorSet<T>
where
    T: Ord,
{
    /// Creates a new, empty SortedVectorSet.
    pub const fn new() -> SortedVectorSet<T> {
        SortedVectorSet(Vec::new())
    }

    /// Creates a new, empty SortedVectorSet, with capacity for `capacity` entries.
    pub fn with_capacity(capacity: usize) -> SortedVectorSet<T> {
        SortedVectorSet(Vec::with_capacity(capacity))
    }

    /// Extracts the inner vector.
    pub fn into_inner(self) -> Vec<T> {
        self.0
    }

    /// Clears the set, removing all elements.
    pub fn clear(&mut self) {
        self.0.clear()
    }

    /// Returns `true` if the set is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Utility function to binary search for an index using the key.
    fn find_index<Q>(&self, q: &Q) -> Result<usize, usize>
    where
        T: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.0.binary_search_by(|e| e.borrow().cmp(q))
    }

    /// Returns `true` if the set contains a value.
    pub fn contains<Q>(&self, q: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.find_index(q).is_ok()
    }

    /// Returns a reference to the value in the set, if any, that is equal to the given value.
    pub fn get<Q>(&self, q: &Q) -> Option<&T>
    where
        T: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.find_index(q).ok().map(|index| &self.0[index])
    }

    /// Utility function for implementing `range` and `range_mut`.
    ///
    /// Convert a range boundary for the start of a range into a slice
    /// index suitable for use in a range expression.
    fn range_index_start<Q>(&self, b: Bound<&Q>) -> usize
    where
        T: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        match b {
            Unbounded => 0,
            Included(q) => match self.find_index(q) {
                Ok(index) => index,
                Err(index) => index,
            },
            Excluded(q) => match self.find_index(q) {
                Ok(index) => index + 1,
                Err(index) => index,
            },
        }
    }

    /// Utility function for implementing `range` and `range_mut`.
    ///
    /// Convert a range boundary for the end of a range into a slice
    /// index suitable for use in a range expression.
    fn range_index_end<Q>(&self, b: Bound<&Q>) -> usize
    where
        T: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        match b {
            Unbounded => self.0.len(),
            Included(q) => match self.find_index(q) {
                Ok(index) => index + 1,
                Err(index) => index,
            },
            Excluded(q) => match self.find_index(q) {
                Ok(index) => index,
                Err(index) => index,
            },
        }
    }

    /// Returns an iterator over the given range of keys.
    ///
    /// # Panics
    ///
    /// Panics if the range start is after the range end.
    pub fn range<Q, R>(&self, range: R) -> std::slice::Iter<T>
    where
        T: Borrow<Q>,
        Q: Ord + ?Sized,
        R: RangeBounds<Q>,
    {
        let start = self.range_index_start(range.start_bound());
        let end = self.range_index_end(range.end_bound());
        if start > end {
            panic!("range start is greater than range end in SortedVectorSet")
        }
        self.0[start..end].iter()
    }

    /// Returns the items that are in `self` that are not in `other`.
    pub fn difference<'a>(&'a self, other: &'a SortedVectorSet<T>) -> Difference<'a, T> {
        Difference(OperationInner {
            left: self.iter().peekable(),
            right: other.iter().peekable(),
        })
    }

    /// Returns the items that are in `self` or `other`, but not in both.
    pub fn symmetric_difference<'a>(
        &'a self,
        other: &'a SortedVectorSet<T>,
    ) -> SymmetricDifference<'a, T> {
        SymmetricDifference(OperationInner {
            left: self.iter().peekable(),
            right: other.iter().peekable(),
        })
    }

    /// Returns the items that are in both `self` and `other`.
    pub fn intersection<'a>(&'a self, other: &'a SortedVectorSet<T>) -> Intersection<'a, T> {
        Intersection(OperationInner {
            left: self.iter().peekable(),
            right: other.iter().peekable(),
        })
    }

    /// Returns the items that are in `self`, `other`, or both.
    pub fn union<'a>(&'a self, other: &'a SortedVectorSet<T>) -> Union<'a, T> {
        Union(OperationInner {
            left: self.iter().peekable(),
            right: other.iter().peekable(),
        })
    }

    /// Returns `true` if `self` has no elements in common with `other`.
    pub fn is_disjoint(&self, other: &SortedVectorSet<T>) -> bool {
        self.intersection(other).next().is_none()
    }

    /// Returns `true` if `self` is a subset of `other`, i.e. `other`
    /// contains at least all values in `self`.
    pub fn is_subset(&self, other: &SortedVectorSet<T>) -> bool {
        other.difference(self).next().is_none()
    }

    /// Returns `true` if `self` is a superset of `other`, i.e. `self`
    /// contains at least all values in `other`.
    pub fn is_superset(&self, other: &SortedVectorSet<T>) -> bool {
        other.is_subset(self)
    }

    /// Adds a value to the set.
    ///
    /// Returns `true` if the set did not already have this value present.
    pub fn insert(&mut self, value: T) -> bool {
        self.replace(value).is_none()
    }

    /// Adds a value to the set, replacing the existing value, if any,
    /// that is equal to the given one.  Returns the replaced value.
    pub fn replace(&mut self, value: T) -> Option<T> {
        let len = self.0.len();
        if len == 0 || self.0[len - 1] < value {
            self.0.push(value);
            None
        } else {
            let mut value = value;
            match self.find_index(&value) {
                Ok(index) => {
                    mem::swap(&mut self.0[index], &mut value);
                    Some(value)
                }
                Err(index) => {
                    self.0.insert(index, value);
                    None
                }
            }
        }
    }

    /// Removes the value in the set, if any, that is equal to the given
    /// one.  Returns `true` if the value was in the set.
    pub fn remove<Q>(&mut self, value: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.take(value).is_some()
    }

    /// Removes and returns the value in the set, if any, that is equal
    /// to the given one.
    pub fn take<Q>(&mut self, value: &Q) -> Option<T>
    where
        T: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        match self.find_index(value) {
            Ok(index) => Some(self.0.remove(index)),
            Err(_index) => None,
        }
    }

    /// Retains only the elements specified by the predicate.
    pub fn retain<F>(&mut self, f: F)
    where
        F: FnMut(&T) -> bool,
    {
        self.0.retain(f)
    }

    /// Moves all elements from `other` into `self`, leaving `other` empty.
    pub fn append(&mut self, other: &mut SortedVectorSet<T>) {
        if other.is_empty() {
            return;
        }

        if self.is_empty() {
            mem::swap(self, other);
            return;
        }

        let self_iter = mem::take(self).into_iter();
        let other_iter = mem::take(other).into_iter();
        let iter = MergeIter::new(self_iter, other_iter);
        self.0 = iter.collect();
    }

    /// Splits the collection in two at the given key.  Returns
    /// everything after the given key, including the key.
    pub fn split_off<Q>(&mut self, q: &Q) -> SortedVectorSet<T>
    where
        T: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let index = match self.find_index(q) {
            Ok(index) => index,
            Err(index) => index,
        };
        SortedVectorSet(self.0.split_off(index))
    }

    /// Returns an iterator over the values in the map, in sorted order
    pub fn iter(&self) -> std::slice::Iter<T> {
        self.0.iter()
    }

    /// Returns the number of elements in the set.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns a reference to the first value in the set, if any.
    pub fn first(&self) -> Option<&T> {
        self.0.first()
    }

    /// Returns a reference to the last value in the set, if any.
    pub fn last(&self) -> Option<&T> {
        self.0.last()
    }

    /// Removes and returns the last value in the set, if any.
    ///
    /// There is no `pop_first` equivalent as removing the first item from a
    /// vector is not efficient.
    pub fn pop_last(&mut self) -> Option<T> {
        self.0.pop()
    }

    /// Extend from a vector of values.  This can be more efficient than
    /// extending from an arbitrary iterator.
    pub fn extend_with_vec(&mut self, mut new: Vec<T>) {
        if new.is_empty() {
            return;
        }
        // Special case for extending with a single item.  This is used by
        // stream-based versions of extend, and it is more efficient to
        // convert back to insert.
        if new.len() == 1 {
            let item = new.into_iter().next().expect("iterator must have one item");
            self.insert(item);
            return;
        }
        // Sort stably so that later duplicates overwrite earlier ones.
        new.sort();
        if self.0.is_empty() {
            // This set is empty, so we can take the new values as-is,
            // removing duplicates if necessary.  In the common case
            // there will be no duplicates, so it's quicker to scan for
            // them first.
            match new.iter().tuple_windows().position(|(a, b)| a == b) {
                Some(first_dup_index) => {
                    // Duplicates start at this index, so deduplicate from
                    // here.
                    let dups = new.split_off(first_dup_index);
                    self.0 = new;
                    self.0.extend(DedupIter::new(dups.into_iter()));
                }
                None => self.0 = new,
            }
            return;
        }
        match (self.0.last(), new.first()) {
            (Some(self_last), Some(new_first)) if self_last < new_first => {
                // All new items are after the end, so we can append them to
                // the vector, after deduplication if necessary.  In the
                // common case there will be no duplicates, so it's quicker to
                // scan for them first.
                match new.iter().tuple_windows().position(|(a, b)| a == b) {
                    Some(first_dup_index) => {
                        // Duplicates start at this index, so deduplicate from
                        // here.
                        let dups = new.split_off(first_dup_index);
                        self.0.extend(new);
                        self.0.extend(DedupIter::new(dups.into_iter()));
                    }
                    None => self.0.extend(new),
                }
            }
            _ => {
                // The vectors must be merged.
                let self_iter = mem::take(self).into_iter();
                let new_iter = new.into_iter();
                self.0 = MergeIter::new(self_iter, new_iter).collect();
            }
        }
    }
}

impl<T> Default for SortedVectorSet<T>
where
    T: Ord,
{
    fn default() -> SortedVectorSet<T> {
        SortedVectorSet::new()
    }
}

impl<T> Debug for SortedVectorSet<T>
where
    T: Ord + Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl<T> IntoIterator for SortedVectorSet<T>
where
    T: Ord,
{
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    #[inline]
    fn into_iter(self) -> std::vec::IntoIter<T> {
        self.0.into_iter()
    }
}

impl<'a, T: 'a> IntoIterator for &'a SortedVectorSet<T>
where
    T: Ord,
{
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    #[inline]
    fn into_iter(self) -> std::slice::Iter<'a, T> {
        self.0.iter()
    }
}

impl<T> Extend<T> for SortedVectorSet<T>
where
    T: Ord,
{
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        let new: Vec<_> = iter.into_iter().collect();
        self.extend_with_vec(new);
    }
}

impl<'a, T> Extend<&'a T> for SortedVectorSet<T>
where
    T: Ord + Copy + 'a,
{
    fn extend<I: IntoIterator<Item = &'a T>>(&mut self, iter: I) {
        let new: Vec<_> = iter.into_iter().copied().collect();
        self.extend_with_vec(new);
    }
}

impl<T> FromIterator<T> for SortedVectorSet<T>
where
    T: Ord,
{
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> SortedVectorSet<T> {
        let iter = iter.into_iter();
        let mut set = SortedVectorSet::new();
        set.extend(iter);
        set
    }
}

struct DedupIter<T, I: Iterator<Item = T>> {
    iter: Peekable<I>,
}

impl<T, I> DedupIter<T, I>
where
    T: Ord,
    I: Iterator<Item = T>,
{
    fn new(iter: I) -> Self {
        DedupIter {
            iter: iter.peekable(),
        }
    }

    fn peek(&mut self) -> Option<&T> {
        self.iter.peek()
    }
}

impl<T, I> Iterator for DedupIter<T, I>
where
    T: Ord,
    I: Iterator<Item = T>,
{
    type Item = T;

    fn next(&mut self) -> Option<T> {
        let mut next = self.iter.next();
        while let (Some(next_ref), Some(after)) = (next.as_ref(), self.iter.peek()) {
            if after > next_ref {
                break;
            }
            next = self.iter.next();
        }
        next
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let (low, high) = self.iter.size_hint();
        (low.min(1), high)
    }
}

struct MergeIter<T, I: Iterator<Item = T>> {
    left: Peekable<I>,
    right: DedupIter<T, I>,
}

impl<T, I> MergeIter<T, I>
where
    T: Ord,
    I: Iterator<Item = T>,
{
    fn new(left: I, right: I) -> Self {
        MergeIter {
            left: left.peekable(),
            right: DedupIter::new(right),
        }
    }
}

impl<T, I> Iterator for MergeIter<T, I>
where
    T: Ord,
    I: Iterator<Item = T>,
{
    type Item = T;

    fn next(&mut self) -> Option<T> {
        let res = match (self.left.peek(), self.right.peek()) {
            (Some(left), Some(right)) => left.cmp(right),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => return None,
        };

        // Check which element comes first and only advance the corresponding
        // iterator.  If the two keys are equal, take the value from `right`.
        // If `right` has multiple equal keys, take the last one.
        match res {
            Ordering::Less => self.left.next(),
            Ordering::Greater => self.right.next(),
            Ordering::Equal => {
                self.left.next();
                self.right.next()
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left_hint = self.left.size_hint();
        let right_hint = self.right.size_hint();
        let low = std::cmp::max(left_hint.0, right_hint.0);
        let high = match (left_hint.1, right_hint.1) {
            (Some(left_high), Some(right_high)) => left_high.checked_add(right_high),
            _ => None,
        };
        (low, high)
    }
}

struct OperationInner<'a, T> {
    left: Peekable<std::slice::Iter<'a, T>>,
    right: Peekable<std::slice::Iter<'a, T>>,
}

impl<'a, T> Iterator for OperationInner<'a, T>
where
    T: Ord,
{
    type Item = (Option<&'a T>, Option<&'a T>);

    fn next(&mut self) -> Option<Self::Item> {
        let res = match (self.left.peek(), self.right.peek()) {
            (Some(left), Some(right)) => left.cmp(right),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => return None,
        };

        // Check which element comes first and only advance the corresponding
        // iterator.  If the two keys are equal, advance both.
        match res {
            Ordering::Less => Some((self.left.next(), None)),
            Ordering::Greater => Some((None, self.right.next())),
            Ordering::Equal => Some((self.left.next(), self.right.next())),
        }
    }
}

pub struct Difference<'a, T: 'a>(OperationInner<'a, T>);

impl<'a, T> Iterator for Difference<'a, T>
where
    T: Ord,
{
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        for next in &mut self.0 {
            match next {
                (Some(left), None) => return Some(left),
                _ => continue,
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left_hint = self.0.left.size_hint();
        let right_hint = self.0.right.size_hint();
        let low = match right_hint.1 {
            Some(right_high) => left_hint.0.saturating_sub(right_high),
            None => 0,
        };
        let high = left_hint.1;
        (low, high)
    }
}

pub struct SymmetricDifference<'a, T: 'a>(OperationInner<'a, T>);

impl<'a, T> Iterator for SymmetricDifference<'a, T>
where
    T: Ord,
{
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        for next in self.0.by_ref() {
            match next {
                (Some(left), None) => return Some(left),
                (None, Some(right)) => return Some(right),
                _ => continue,
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left_hint = self.0.left.size_hint();
        let right_hint = self.0.right.size_hint();
        let low = 0;
        let high = match (left_hint.1, right_hint.1) {
            (Some(left_high), Some(right_high)) => left_high.checked_add(right_high),
            _ => None,
        };
        (low, high)
    }
}

pub struct Intersection<'a, T: 'a>(OperationInner<'a, T>);

impl<'a, T> Iterator for Intersection<'a, T>
where
    T: Ord,
{
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        for next in self.0.by_ref() {
            match next {
                (Some(left), Some(_right)) => return Some(left),
                _ => continue,
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left_hint = self.0.left.size_hint();
        let right_hint = self.0.right.size_hint();
        let low = 0;
        let high = match (left_hint.1, right_hint.1) {
            (Some(left_high), Some(right_high)) => Some(std::cmp::min(left_high, right_high)),
            _ => None,
        };
        (low, high)
    }
}

pub struct Union<'a, T: 'a>(OperationInner<'a, T>);

impl<'a, T> Iterator for Union<'a, T>
where
    T: Ord,
{
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        for next in self.0.by_ref() {
            match next {
                (_, Some(right)) => return Some(right),
                (Some(left), None) => return Some(left),
                _ => continue,
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left_hint = self.0.left.size_hint();
        let right_hint = self.0.right.size_hint();
        let low = std::cmp::max(left_hint.0, right_hint.0);
        let high = match (left_hint.1, right_hint.1) {
            (Some(left_high), Some(right_high)) => left_high.checked_add(right_high),
            _ => None,
        };
        (low, high)
    }
}

impl<T> BitAnd<&SortedVectorSet<T>> for &SortedVectorSet<T>
where
    T: Ord + Clone,
{
    type Output = SortedVectorSet<T>;

    fn bitand(self, rhs: &SortedVectorSet<T>) -> SortedVectorSet<T> {
        self.intersection(rhs).cloned().collect()
    }
}

impl<T> Sub<&SortedVectorSet<T>> for &SortedVectorSet<T>
where
    T: Ord + Clone,
{
    type Output = SortedVectorSet<T>;

    fn sub(self, rhs: &SortedVectorSet<T>) -> SortedVectorSet<T> {
        self.difference(rhs).cloned().collect()
    }
}

impl<T> BitXor<&SortedVectorSet<T>> for &SortedVectorSet<T>
where
    T: Ord + Clone,
{
    type Output = SortedVectorSet<T>;

    fn bitxor(self, rhs: &SortedVectorSet<T>) -> SortedVectorSet<T> {
        self.symmetric_difference(rhs).cloned().collect()
    }
}

impl<T> BitOr<&SortedVectorSet<T>> for &SortedVectorSet<T>
where
    T: Ord + Clone,
{
    type Output = SortedVectorSet<T>;

    fn bitor(self, rhs: &SortedVectorSet<T>) -> SortedVectorSet<T> {
        self.union(rhs).cloned().collect()
    }
}

impl<T> From<BTreeSet<T>> for SortedVectorSet<T> {
    fn from(bset: BTreeSet<T>) -> SortedVectorSet<T> {
        // The BTreeSet will iterate in sorted order.
        let v = bset.into_iter().collect();
        SortedVectorSet(v)
    }
}

impl<T> Arbitrary for SortedVectorSet<T>
where
    T: Arbitrary + Ord,
{
    fn arbitrary(g: &mut Gen) -> SortedVectorSet<T> {
        let vec: Vec<T> = Arbitrary::arbitrary(g);
        vec.into_iter().collect()
    }

    fn shrink(&self) -> Box<dyn Iterator<Item = SortedVectorSet<T>>> {
        let vec: Vec<T> = self.clone().into_iter().collect();
        Box::new(
            vec.shrink()
                .map(|v| v.into_iter().collect::<SortedVectorSet<T>>()),
        )
    }
}

#[macro_export]
macro_rules! sorted_vector_set {
    ( $( $value:expr_2021 ),* $( , )? ) => {
        {
            let size = <[()]>::len(&[ $( $crate::replace_expr!( ( $value ) () ) ),* ]);
            let mut set = $crate::SortedVectorSet::with_capacity(size);
            $(
                set.insert($value);
            )*
            set
        }
    };
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use quickcheck::quickcheck;

    use super::*;

    #[test]
    fn insert_contains_take_remove() {
        let mut svs = SortedVectorSet::new();
        assert!(svs.insert("test1"));
        assert!(svs.insert("test2"));
        assert!(svs.insert("test4"));
        assert!(svs.insert("test3"));
        assert!(!svs.insert("test1"));
        assert!(svs.contains(&"test1"));
        assert!(!svs.contains(&"never"));
        assert_eq!(svs.take(&"test3"), Some("test3"));
        assert_eq!(svs.take(&"never"), None);
        assert!(svs.remove(&"test2"));
        assert!(!svs.remove(&"test2"));
        assert!(!svs.remove(&"never"));
    }

    #[test]
    fn iter() {
        let svs = sorted_vector_set! { 1, 2, 3, 4, 5 };
        let mut i = svs.iter();
        assert_eq!(i.next(), Some(&1));
        assert_eq!(i.next(), Some(&2));
        assert_eq!(i.next(), Some(&3));
        assert_eq!(i.next(), Some(&4));
        assert_eq!(i.next(), Some(&5));
        assert_eq!(i.next(), None);
        let mut i = svs.into_iter();
        assert_eq!(i.next(), Some(1));
        assert_eq!(i.next(), Some(2));
        assert_eq!(i.next(), Some(3));
        assert_eq!(i.next(), Some(4));
        assert_eq!(i.next(), Some(5));
        assert_eq!(i.next(), None);
    }

    #[test]
    fn range() {
        let svs = sorted_vector_set! { 1, 3, 5, 7, 9, 11};
        assert_eq!(svs.range(3..9).cloned().collect::<Vec<_>>(), vec![3, 5, 7]);
        assert_eq!(svs.range(3..=7).cloned().collect::<Vec<_>>(), vec![3, 5, 7]);
        assert_eq!(svs.range(..2).cloned().collect::<Vec<_>>(), vec![1]);
        assert_eq!(svs.range(6..).cloned().collect::<Vec<_>>(), vec![7, 9, 11]);
    }

    #[test]
    fn first_last() {
        let mut svs = sorted_vector_set! { 5, 10, 15, 20 };
        assert_eq!(svs.first(), Some(&5));
        assert_eq!(svs.last(), Some(&20));
        assert_eq!(svs.pop_last(), Some(20));
        assert_eq!(svs.last(), Some(&15));
        assert_eq!(svs.pop_last(), Some(15));
        assert_eq!(svs.pop_last(), Some(10));
        assert_eq!(svs.first(), Some(&5));
        assert_eq!(svs.last(), Some(&5));
        assert_eq!(svs.pop_last(), Some(5));
        assert_eq!(svs.pop_last(), None);
        assert_eq!(svs.first(), None);
        assert_eq!(svs.last(), None);
    }

    #[test]
    fn retain() {
        let mut svs = sorted_vector_set! {
            1, 2, 3, 4, 5
        };
        svs.retain(|v| *v != 3);
        assert_eq!(svs, sorted_vector_set! { 1, 2, 4, 5 });
    }

    #[test]
    fn split_off_append_extend() {
        let mut svs = sorted_vector_set! { 1, 3, 5, 7, 9, 11};
        let mut svs2 = svs.split_off(&7);
        assert_eq!(svs.iter().cloned().collect::<Vec<_>>(), vec![1, 3, 5]);
        assert_eq!(svs2.iter().cloned().collect::<Vec<_>>(), vec![7, 9, 11]);
        svs2.extend(vec![4, 5, 6, 7, 8]);
        assert_eq!(
            svs2.iter().cloned().collect::<Vec<_>>(),
            vec![4, 5, 6, 7, 8, 9, 11]
        );
        svs2.append(&mut svs);
        assert!(svs.is_empty());
        assert_eq!(
            svs2.iter().cloned().collect::<Vec<_>>(),
            vec![1, 3, 4, 5, 6, 7, 8, 9, 11]
        );
    }

    #[test]
    fn extend_optimizations() {
        // Initializing via extend will sort and take the values.
        let mut svs = SortedVectorSet::new();
        svs.extend(vec![3, 2, 1]);

        assert_eq!(svs.iter().cloned().collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(svs.first(), Some(&1));

        // This also works if there are duplicates: the last value will be
        // taken.
        let mut svs = SortedVectorSet::new();
        svs.extend(vec![3, 2, 1, 6, 4, 5, 1, 6]);
        assert_eq!(
            svs.iter().cloned().collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5, 6],
        );
        assert_eq!(svs.first(), Some(&1));
        assert_eq!(svs.pop_last(), Some(6));

        // Extending with values that are all after the highest value will
        // efficiently append to the vector.
        svs.extend(vec![9, 7, 8, 6]);

        assert_eq!(
            svs.iter().cloned().collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9]
        );
        assert_eq!(svs.last(), Some(&9));

        // If there are duplicate values, then the last value will be taken.
        svs.extend(vec![11, 12, 10, 12]);

        assert_eq!(
            svs.iter().cloned().collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
        );
    }

    #[test]
    fn intersect_difference_symdiff_union() {
        let svs1 = sorted_vector_set! { 1, 3, 4, 5, 6, 7, 9 };
        let svs2 = sorted_vector_set! { 2, 4, 5, 6, 7, 8, 10};
        assert_eq!(
            svs1.intersection(&svs2)
                .cloned()
                .collect::<SortedVectorSet<_>>(),
            sorted_vector_set! { 4, 5, 6, 7 },
        );
        assert_eq!(
            svs1.difference(&svs2)
                .cloned()
                .collect::<SortedVectorSet<_>>(),
            sorted_vector_set! { 1, 3, 9 },
        );
        assert_eq!(
            svs1.symmetric_difference(&svs2)
                .cloned()
                .collect::<SortedVectorSet<_>>(),
            sorted_vector_set! { 1, 2, 3, 8, 9, 10 },
        );
        assert_eq!(
            svs1.union(&svs2).cloned().collect::<SortedVectorSet<_>>(),
            sorted_vector_set! { 1, 2, 3, 4, 5, 6, 7, 8, 9, 10 },
        );
        assert_eq!(&svs1 & &svs2, sorted_vector_set! { 4, 5, 6, 7 },);
        assert_eq!(&svs1 - &svs2, sorted_vector_set! { 1, 3, 9 },);
        assert_eq!(&svs1 ^ &svs2, sorted_vector_set! { 1, 2, 3, 8, 9, 10 },);
        assert_eq!(&svs1 | &svs2, (1..=10).collect(),);
    }

    #[test]
    fn debug_print() {
        assert_eq!(&format!("{:?}", SortedVectorSet::<i32>::new()), "{}");
        assert_eq!(
            &format!("{:?}", sorted_vector_set! {1, 10, 100}),
            "{1, 10, 100}"
        );
    }

    fn svset_from_btreeset<T: Ord + Clone>(b: &BTreeSet<T>) -> SortedVectorSet<T> {
        let mut svs = SortedVectorSet::with_capacity(b.len());
        for v in b.iter() {
            svs.insert(v.clone());
        }
        svs
    }

    quickcheck! {
        fn like_btreeset_is_empty(b: BTreeSet<u32>) -> bool {
            let svs = svset_from_btreeset(&b);
            svs.is_empty() == b.is_empty()
        }

        fn like_btreeset_len(b: BTreeSet<u32>) -> bool {
            let svs = svset_from_btreeset(&b);
            svs.len() == b.len()
        }

        fn like_btreeset_iter(b: BTreeSet<u32>) -> bool {
            let svs = svset_from_btreeset(&b);
            itertools::equal(svs.iter(), b.iter())
        }

        fn roundtrip_via_btreeset(svs1: SortedVectorSet<u32>) -> bool {
            let b: BTreeSet<u32> = svs1.clone().into_iter().collect();
            let svs2: SortedVectorSet<u32> = b.into();
            itertools::equal(svs1, svs2)
        }
    }
}
