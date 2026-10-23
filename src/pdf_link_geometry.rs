//! PDF link regions in default user space. No outward bounding-box clipping:
//! non-convex paths and holes retain their actual active regions as quads.

const EPS: f64 = 1.0e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub(crate) fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    fn midpoint(self, other: Self) -> Self {
        Self::new((self.x + other.x) * 0.5, (self.y + other.y) * 0.5)
    }

    fn finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Matrix(pub [f64; 6]);

impl Matrix {
    pub(crate) const IDENTITY: Self = Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// PDF CTM concatenation: map through `next`, then through `self`.
    pub(crate) fn then(self, next: Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [g, h, i, j, k, l] = next.0;
        Self([
            a * g + c * h,
            b * g + d * h,
            a * i + c * j,
            b * i + d * j,
            a * k + c * l + e,
            b * k + d * l + f,
        ])
    }

    pub(crate) fn point(self, point: Point) -> Point {
        let [a, b, c, d, e, f] = self.0;
        Point::new(a * point.x + c * point.y + e, b * point.x + d * point.y + f)
    }
}

fn cross(a: Point, b: Point, p: Point) -> f64 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

fn area(points: &[Point]) -> f64 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum::<f64>()
        * 0.5
}

/// Counterclockwise vertices, as required for link annotation QuadPoints.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Quad(pub [Point; 4]);

impl Quad {
    pub(crate) fn new(mut points: [Point; 4]) -> Option<Self> {
        if points.iter().any(|point| !point.finite()) {
            return None;
        }
        let signed_area = area(&points);
        if signed_area.abs() <= EPS {
            return None;
        }
        if signed_area < 0.0 {
            points = [points[1], points[0], points[3], points[2]];
        }
        for index in 0..4 {
            let a = points[index];
            let b = points[(index + 1) % 4];
            let c = points[(index + 2) % 4];
            if (a.x - b.x).abs() <= EPS && (a.y - b.y).abs() <= EPS {
                return None;
            }
            if cross(a, b, c) < -EPS {
                return None;
            }
        }
        Some(Self(points))
    }

    pub(crate) fn rectangle(
        matrix: Matrix,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> Option<Self> {
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        Self::new([
            matrix.point(Point::new(x, y)),
            matrix.point(Point::new(x + width, y)),
            matrix.point(Point::new(x + width, y + height)),
            matrix.point(Point::new(x, y + height)),
        ])
    }

    #[cfg(test)]
    pub(crate) fn contains(self, point: Point) -> bool {
        (0..4).all(|index| cross(self.0[index], self.0[(index + 1) % 4], point) >= -EPS)
    }

    fn axis_aligned_bounds(self) -> Option<[f64; 4]> {
        if !(0..4).all(|i| {
            let (a, b) = (self.0[i], self.0[(i + 1) % 4]);
            (a.x - b.x).abs() <= EPS || (a.y - b.y).abs() <= EPS
        }) {
            return None;
        }
        Region::quad(self).bounds()
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Region {
    pub quads: Vec<Quad>,
}

impl Region {
    pub(crate) fn quad(quad: Quad) -> Self {
        Self { quads: vec![quad] }
    }

    pub(crate) fn bounds(&self) -> Option<[f64; 4]> {
        let first = self.quads.first()?.0[0];
        let mut bounds = [first.x, first.y, first.x, first.y];
        for point in self.quads.iter().flat_map(|quad| quad.0) {
            bounds[0] = bounds[0].min(point.x);
            bounds[1] = bounds[1].min(point.y);
            bounds[2] = bounds[2].max(point.x);
            bounds[3] = bounds[3].max(point.y);
        }
        Some(bounds)
    }

    pub(crate) fn intersect(&self, other: &Self) -> Self {
        let mut quads = Vec::new();
        for subject in &self.quads {
            for clip in &other.quads {
                append_convex_quads(&mut quads, convex_intersection(subject, clip));
            }
        }
        Self { quads }
    }

    /// Partition activation without joining empty areas into one Rect. PDFium
    /// hit-tests Rect alone, so disjoint lines and orthogonal clip holes need
    /// separate annotations. Sweep the rectangle union to remove overlapping
    /// text/space/padding boxes, then join vertically identical intervals.
    /// Non-axis-aligned quads retain their exact standard QuadPoints in their
    /// own annotations; a reader must honor QuadPoints for their sloped edges.
    pub(crate) fn annotation_regions(&self) -> Vec<Self> {
        let mut rectangles = Vec::new();
        let mut other = Vec::new();
        for &quad in &self.quads {
            if let Some(bounds) = quad.axis_aligned_bounds() {
                rectangles.push(bounds);
            } else {
                other.push(quad);
            }
        }
        let mut events = Vec::with_capacity(rectangles.len() * 2);
        let mut levels = Vec::with_capacity(rectangles.len() * 2);
        for (index, rect) in rectangles.iter().enumerate() {
            events.push((rect[1], index, true));
            events.push((rect[3], index, false));
            levels.extend([rect[1], rect[3]]);
        }
        events.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        levels.sort_by(f64::total_cmp);
        levels.dedup_by(|a, b| (*a - *b).abs() <= EPS);
        let mut active = std::collections::BTreeSet::new();
        let mut event = 0;
        let mut merged = Vec::<[f64; 4]>::new();
        let mut previous = Vec::<(f64, f64, usize)>::new();
        for band in levels.windows(2) {
            let (low, high) = (band[0], band[1]);
            while event < events.len() && events[event].0 <= low + EPS {
                let (_, index, begin) = events[event];
                if begin {
                    active.insert(index);
                } else {
                    active.remove(&index);
                }
                event += 1;
            }
            let mut intervals: Vec<_> = active
                .iter()
                .map(|&i| (rectangles[i][0], rectangles[i][2]))
                .collect();
            intervals.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
            let mut union = Vec::<(f64, f64)>::new();
            for (left, right) in intervals {
                if let Some(last) = union.last_mut().filter(|last| left <= last.1 + EPS) {
                    last.1 = last.1.max(right);
                } else {
                    union.push((left, right));
                }
            }
            let mut next = Vec::with_capacity(union.len());
            let mut old = 0;
            for (left, right) in union {
                while old < previous.len() && previous[old].0 < left - EPS {
                    old += 1;
                }
                let index = if let Some(&(_, _, index)) = previous.get(old).filter(|&&(a, b, i)| {
                    (a - left).abs() <= EPS
                        && (b - right).abs() <= EPS
                        && (merged[i][3] - low).abs() <= EPS
                }) {
                    merged[index][3] = high;
                    index
                } else {
                    let index = merged.len();
                    merged.push([left, low, right, high]);
                    index
                };
                next.push((left, right, index));
            }
            previous = next;
        }
        // A padding rectangle can already cover a transformed inner glyph.
        other.retain(|quad| {
            !merged.iter().any(|r| {
                quad.0.iter().all(|p| {
                    p.x >= r[0] - EPS && p.x <= r[2] + EPS && p.y >= r[1] - EPS && p.y <= r[3] + EPS
                })
            })
        });
        let mut result: Vec<_> = merged
            .into_iter()
            .filter_map(|[l, b, r, t]| {
                Quad::rectangle(Matrix::IDENTITY, l, b, r - l, t - b).map(Self::quad)
            })
            .collect();
        result.extend(other.into_iter().map(Self::quad));
        result
    }

    /// Tessellate closed contours with the PDF nonzero or even-odd fill rule.
    /// Split bands at every endpoint and crossing so their edge order is fixed.
    /// Each filled interval is a trapezoid; holes remain empty intervals.
    pub(crate) fn from_contours(contours: &[Vec<Point>], evenodd: bool) -> Self {
        let mut edges = Vec::new();
        let mut levels = Vec::new();
        for contour in contours {
            if contour.len() < 3 || contour.iter().any(|point| !point.finite()) {
                continue;
            }
            for (a, b) in contour
                .iter()
                .zip(contour.iter().cycle().skip(1))
                .take(contour.len())
            {
                levels.push(a.y);
                if (b.y - a.y).abs() > EPS {
                    edges.push(Edge { a: *a, b: *b });
                }
            }
        }
        for (index, a) in edges.iter().enumerate() {
            for b in &edges[index + 1..] {
                if let Some(y) = a.crossing_y(*b) {
                    levels.push(y);
                }
            }
        }
        levels.sort_by(f64::total_cmp);
        levels.dedup_by(|a, b| (*a - *b).abs() <= EPS);
        let mut quads = Vec::new();
        for band in levels.windows(2) {
            let (low, high) = (band[0], band[1]);
            if high - low <= EPS {
                continue;
            }
            let middle = (low + high) * 0.5;
            let mut active = edges
                .iter()
                .filter(|edge| edge.a.y.min(edge.b.y) < middle && edge.a.y.max(edge.b.y) > middle)
                .map(|edge| (edge.x_at(middle), edge))
                .collect::<Vec<_>>();
            active.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut winding = 0_i32;
            let mut index = 0;
            let mut left: Option<&Edge> = None;
            while index < active.len() {
                let x = active[index].0;
                let edge = active[index].1;
                let inside = if evenodd {
                    winding.rem_euclid(2) != 0
                } else {
                    winding != 0
                };
                while index < active.len() && (active[index].0 - x).abs() <= EPS {
                    winding += if active[index].1.b.y > active[index].1.a.y {
                        1
                    } else {
                        -1
                    };
                    index += 1;
                }
                let next_inside = if evenodd {
                    winding.rem_euclid(2) != 0
                } else {
                    winding != 0
                };
                if !inside && next_inside {
                    left = Some(edge);
                } else if inside && !next_inside {
                    if let Some(left) = left.take() {
                        append_convex_quads(
                            &mut quads,
                            vec![
                                Point::new(left.x_at(low), low),
                                Point::new(edge.x_at(low), low),
                                Point::new(edge.x_at(high), high),
                                Point::new(left.x_at(high), high),
                            ],
                        );
                    }
                }
            }
        }
        Self { quads }
    }
}

#[derive(Clone, Copy)]
struct Edge {
    a: Point,
    b: Point,
}

impl Edge {
    fn x_at(self, y: f64) -> f64 {
        self.a.x + (y - self.a.y) * (self.b.x - self.a.x) / (self.b.y - self.a.y)
    }

    fn crossing_y(self, other: Self) -> Option<f64> {
        let a = self.b.x - self.a.x;
        let b = self.b.y - self.a.y;
        let c = other.b.x - other.a.x;
        let d = other.b.y - other.a.y;
        let denominator = a * d - b * c;
        if denominator.abs() <= EPS {
            return None;
        }
        let e = other.a.x - self.a.x;
        let f = other.a.y - self.a.y;
        let t = (e * d - f * c) / denominator;
        let u = (e * b - f * a) / denominator;
        if t > EPS && t < 1.0 - EPS && u > EPS && u < 1.0 - EPS {
            Some(self.a.y + t * b)
        } else {
            None
        }
    }
}

fn convex_intersection(subject: &Quad, clip: &Quad) -> Vec<Point> {
    let mut points = subject.0.to_vec();
    for index in 0..4 {
        if points.is_empty() {
            break;
        }
        let a = clip.0[index];
        let b = clip.0[(index + 1) % 4];
        let mut result = Vec::new();
        let mut previous = *points.last().unwrap();
        let mut previous_side = cross(a, b, previous);
        for current in &points {
            let side = cross(a, b, *current);
            if (previous_side >= -EPS) != (side >= -EPS) {
                let fraction = previous_side / (previous_side - side);
                result.push(Point::new(
                    previous.x + fraction * (current.x - previous.x),
                    previous.y + fraction * (current.y - previous.y),
                ));
            }
            if side >= -EPS {
                result.push(*current);
            }
            previous = *current;
            previous_side = side;
        }
        result.dedup_by(|a, b| (a.x - b.x).abs() <= EPS && (a.y - b.y).abs() <= EPS);
        if result.len() > 1 {
            let first = result[0];
            let last = result[result.len() - 1];
            if (first.x - last.x).abs() <= EPS && (first.y - last.y).abs() <= EPS {
                result.pop();
            }
        }
        points = result;
    }
    points
}

fn append_convex_quads(out: &mut Vec<Quad>, mut points: Vec<Point>) {
    points.dedup_by(|a, b| (a.x - b.x).abs() <= EPS && (a.y - b.y).abs() <= EPS);
    if points.len() > 1 {
        let first = points[0];
        let last = points[points.len() - 1];
        if (first.x - last.x).abs() <= EPS && (first.y - last.y).abs() <= EPS {
            points.pop();
        }
    }
    if points.len() < 3 || area(&points).abs() <= EPS {
        return;
    }
    if points.len() == 4 {
        if let Some(quad) = Quad::new([points[0], points[1], points[2], points[3]]) {
            out.push(quad);
        }
        return;
    }
    // Partition each fan triangle into three genuine quadrilaterals. Repeating
    // a triangle vertex would create a degenerate QuadPoints activation cell.
    for index in 1..points.len() - 1 {
        let (a, b, c) = (points[0], points[index], points[index + 1]);
        let center = Point::new((a.x + b.x + c.x) / 3.0, (a.y + b.y + c.y) / 3.0);
        for vertices in [
            [a, a.midpoint(b), center, c.midpoint(a)],
            [b, b.midpoint(c), center, a.midpoint(b)],
            [c, c.midpoint(a), center, b.midpoint(c)],
        ] {
            if let Some(quad) = Quad::new(vertices) {
                out.push(quad);
            }
        }
    }
}

/// Flatten a cubic in final page coordinates, so the error bound does not grow
/// with a later CSS scale. The output appends endpoints, excluding `start`.
pub(crate) fn flatten_cubic(
    start: Point,
    a: Point,
    b: Point,
    end: Point,
    tolerance: f64,
    out: &mut Vec<Point>,
) {
    if ![start, a, b, end].iter().all(|point| point.finite())
        || !tolerance.is_finite()
        || tolerance <= 0.0
    {
        return;
    }
    let mut pending = vec![(start, a, b, end, 0_u8)];
    while let Some((start, a, b, end, depth)) = pending.pop() {
        if depth >= 20
            || (distance_to_segment(a, start, end) <= tolerance
                && distance_to_segment(b, start, end) <= tolerance)
        {
            out.push(end);
            continue;
        }
        let ab = start.midpoint(a);
        let bc = a.midpoint(b);
        let cd = b.midpoint(end);
        let left = ab.midpoint(bc);
        let right = bc.midpoint(cd);
        let middle = left.midpoint(right);
        pending.push((middle, right, cd, end, depth + 1));
        pending.push((start, ab, left, middle, depth + 1));
    }
}

fn distance_to_segment(point: Point, a: Point, b: Point) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let squared = dx * dx + dy * dy;
    let t = if squared > EPS * EPS {
        (((point.x - a.x) * dx + (point.y - a.y) * dy) / squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (point.x - a.x - t * dx).hypot(point.y - a.y - t * dy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(x: f64, y: f64, w: f64, h: f64) -> Quad {
        Quad::rectangle(Matrix::IDENTITY, x, y, w, h).unwrap()
    }

    fn covered(region: &Region, x: f64, y: f64) -> bool {
        region
            .quads
            .iter()
            .any(|quad| quad.contains(Point::new(x, y)))
    }

    fn region_area(region: &Region) -> f64 {
        region.quads.iter().map(|quad| area(&quad.0)).sum()
    }

    #[test]
    fn pdf_link_geometry_affine_order_and_reflection_keep_counterclockwise_regions() {
        let translate = Matrix([1.0, 0.0, 0.0, 1.0, 20.0, 30.0]);
        let scale = Matrix([-2.0, 0.0, 0.0, 3.0, 0.0, 0.0]);
        let transformed = Quad::rectangle(translate.then(scale), 1.0, 2.0, 4.0, 5.0).unwrap();
        assert_eq!(
            Region::quad(transformed).bounds(),
            Some([10.0, 36.0, 18.0, 51.0])
        );
        assert!((area(&transformed.0) - 120.0).abs() < EPS);
        assert!(transformed.contains(Point::new(14.0, 40.0)));
    }

    #[test]
    fn pdf_link_geometry_rectangle_clipping_does_not_expand_to_ink_bounds() {
        let actual = Region::quad(rectangle(0.0, 0.0, 20.0, 20.0))
            .intersect(&Region::quad(rectangle(5.0, 8.0, 10.0, 4.0)));
        assert_eq!(actual.quads.len(), 1);
        assert_eq!(actual.bounds(), Some([5.0, 8.0, 15.0, 12.0]));
        assert_eq!(region_area(&actual), 40.0);
    }

    #[test]
    fn pdf_link_geometry_rotated_clipping_retains_only_the_intersection() {
        let diamond = Quad::new([
            Point::new(0.0, 5.0),
            Point::new(5.0, 0.0),
            Point::new(10.0, 5.0),
            Point::new(5.0, 10.0),
        ])
        .unwrap();
        let clip = rectangle(0.0, 0.0, 7.0, 7.0);
        let actual = Region::quad(diamond).intersect(&Region::quad(clip));
        // Diamond area 50, minus two clipped corner triangles of area 9,
        // plus their overlapping half-unit triangle: 50 - 9 - 9 + 0.5.
        assert!((region_area(&actual) - 32.5).abs() < EPS);
        for ix in 0..20 {
            for iy in 0..20 {
                let p = Point::new((ix as f64 + 0.37) * 0.5, (iy as f64 + 0.19) * 0.5);
                assert_eq!(
                    covered(&actual, p.x, p.y),
                    diamond.contains(p) && clip.contains(p),
                    "{p:?}"
                );
            }
        }
        assert!(actual.quads.iter().all(|quad| area(&quad.0) > EPS));
    }

    #[test]
    fn pdf_link_geometry_path_holes_follow_the_selected_fill_rule() {
        let outer = rectangle(0.0, 0.0, 10.0, 10.0).0.to_vec();
        let inner = rectangle(3.0, 3.0, 4.0, 4.0).0.to_vec();
        let evenodd = Region::from_contours(&[outer.clone(), inner.clone()], true);
        let nonzero = Region::from_contours(&[outer.clone(), inner.clone()], false);
        let mut reversed = inner;
        reversed.reverse();
        let nonzero_hole = Region::from_contours(&[outer, reversed], false);
        assert_eq!(region_area(&evenodd), 84.0);
        assert_eq!(region_area(&nonzero), 100.0);
        assert_eq!(region_area(&nonzero_hole), 84.0);
        assert!(!covered(&evenodd, 5.0, 5.0));
        assert!(!covered(&nonzero_hole, 5.0, 5.0));
        assert!(covered(&nonzero, 5.0, 5.0));
    }

    #[test]
    fn pdf_link_geometry_concave_path_and_crossings_keep_gaps_inactive() {
        let shape = vec![
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(10.0, 4.0),
            Point::new(4.0, 4.0),
            Point::new(4.0, 10.0),
            Point::new(0.0, 10.0),
        ];
        let region = Region::from_contours(&[shape], false);
        assert_eq!(region_area(&region), 64.0);
        assert!(!covered(&region, 8.0, 8.0));
        let bow_tie = vec![
            Point::new(0.0, 0.0),
            Point::new(10.0, 10.0),
            Point::new(0.0, 10.0),
            Point::new(10.0, 0.0),
        ];
        let crossing = Region::from_contours(&[bow_tie], true);
        assert_eq!(region_area(&crossing), 50.0);
        assert!(covered(&crossing, 5.0, 2.0));
        assert!(!covered(&crossing, 1.0, 5.0));
        assert!(
            crossing
                .quads
                .iter()
                .all(|quad| Quad::new(quad.0).is_some())
        );
    }

    #[test]
    fn pdf_link_geometry_annotation_rects_preserve_holes_and_merge_padding() {
        let mut region = Region {
            quads: vec![
                rectangle(0.0, 0.0, 100.0, 40.0),
                rectangle(0.0, 40.0, 40.0, 60.0),
                rectangle(10.0, 10.0, 20.0, 12.0), // text inside padding
                rectangle(120.0, 0.0, 40.0, 100.0), // disconnected box
            ],
        };
        let regions = region.annotation_regions();
        assert_eq!(regions.len(), 3);
        for x in 0..180 {
            for y in 0..120 {
                let p = Point::new(x as f64 + 0.5, y as f64 + 0.5);
                let rect_hit = regions.iter().any(|r| {
                    let [l, b, r, t] = r.bounds().unwrap();
                    p.x >= l && p.x <= r && p.y >= b && p.y <= t
                });
                let expected = (p.x < 100.0 && p.y < 40.0)
                    || (p.x < 40.0 && p.y < 100.0)
                    || (p.x > 120.0 && p.x < 160.0 && p.y < 100.0);
                assert_eq!(rect_hit, expected, "{p:?}");
            }
        }
        region.quads.push(rectangle(0.0, 0.0, 160.0, 100.0));
        let padded = region.annotation_regions();
        assert_eq!(padded.len(), 1);
        assert_eq!(padded[0].bounds(), Some([0.0, 0.0, 160.0, 100.0]));
        let ring = Region::from_contours(
            &[
                rectangle(0.0, 0.0, 10.0, 10.0).0.to_vec(),
                rectangle(3.0, 3.0, 4.0, 4.0).0.to_vec(),
            ],
            true,
        );
        let pieces = ring.annotation_regions();
        assert_eq!(pieces.len(), 4);
        assert!(!pieces.iter().any(|p| covered(p, 5.0, 5.0)));
        assert_eq!(pieces.iter().map(region_area).sum::<f64>(), 84.0);
    }

    #[test]
    fn pdf_link_geometry_annotation_regions_keep_sloped_quadpoints() {
        let diamond = Quad::new([
            Point::new(0.0, 5.0),
            Point::new(5.0, 0.0),
            Point::new(10.0, 5.0),
            Point::new(5.0, 10.0),
        ])
        .unwrap();
        let region = Region::quad(diamond);
        let annotations = region.annotation_regions();
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].quads, vec![diamond]);
        assert!(!covered(&annotations[0], 1.0, 1.0));
    }

    #[test]
    fn pdf_link_geometry_empty_and_nonfinite_regions_do_not_create_actions() {
        assert!(Quad::rectangle(Matrix::IDENTITY, 0.0, 0.0, 0.0, 10.0).is_none());
        assert!(Quad::rectangle(Matrix::IDENTITY, f64::NAN, 0.0, 10.0, 10.0).is_none());
        let empty = Region::quad(rectangle(0.0, 0.0, 10.0, 10.0))
            .intersect(&Region::quad(rectangle(20.0, 20.0, 5.0, 5.0)));
        assert!(empty.quads.is_empty());
        assert!(empty.bounds().is_none());
    }

    #[test]
    fn pdf_link_geometry_cubic_flattening_bounds_page_space_error() {
        let start = Point::new(0.0, 0.0);
        let a = Point::new(0.0, 100.0);
        let b = Point::new(100.0, 100.0);
        let end = Point::new(100.0, 0.0);
        let mut polyline = vec![start];
        flatten_cubic(start, a, b, end, 0.01, &mut polyline);
        assert_eq!(polyline.last(), Some(&end));
        for i in 0..1001 {
            let t = i as f64 / 1000.0;
            let u = 1.0 - t;
            let point = Point::new(
                u * u * u * start.x
                    + 3.0 * u * u * t * a.x
                    + 3.0 * u * t * t * b.x
                    + t * t * t * end.x,
                u * u * u * start.y
                    + 3.0 * u * u * t * a.y
                    + 3.0 * u * t * t * b.y
                    + t * t * t * end.y,
            );
            let error = polyline
                .windows(2)
                .map(|pair| distance_to_segment(point, pair[0], pair[1]))
                .fold(f64::INFINITY, f64::min);
            assert!(error <= 0.01, "{t}: {error}");
        }
    }

    #[test]
    fn pdf_link_geometry_collinear_cubic_reversal_is_not_discarded() {
        let mut points = vec![Point::new(0.0, 0.0)];
        flatten_cubic(
            points[0],
            Point::new(100.0, 0.0),
            Point::new(-100.0, 0.0),
            Point::new(0.0, 0.0),
            0.01,
            &mut points,
        );
        assert!(points.iter().any(|point| point.x > 28.8));
        assert!(points.iter().any(|point| point.x < -28.8));
    }
}
