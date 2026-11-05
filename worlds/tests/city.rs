//! The city: where the camera starts.

use worldgen::generator::generate;
use worldgen::grid::Box3;
use worldgen::units::m;
use worlds::city::city;

#[test]
fn starts_at_eye_height_on_the_road_looking_through_an_open_entrance_into_a_room() {
    for seed in [42, 1, 2, 3] {
        let world = city(seed);
        let [x, y, z] = world.spawn;
        let grid = generate(&world.generator, &Box3 { min: [x - 8, y - m(2.0), z], max: [x + 8, y + m(3.0), z + m(16.0)] }, 0);
        assert_ne!(grid.get(x, y - m(1.75), z), 0, "seed {seed}: the road under the eyes");
        // The building's front: the first thing ahead 2 m above the eyes, over the doorway. Steps of 2 keep to sites.
        let front = (z..z + m(16.0)).step_by(2).find(|&z| grid.get(x, y + m(2.0), z) != 0).expect("a front ahead");
        // From eye height to the top of the door: clear through the doorway and 3 m into the room.
        for z in (z..front + m(3.0)).step_by(2) {
            for dy in [0, 2, 4] {
                assert_eq!(grid.get(x, y + dy, z), 0, "seed {seed}: something at z = {z}, {dy} over the eyes");
            }
        }
    }
}
