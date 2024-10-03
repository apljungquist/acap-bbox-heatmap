use std::ffi::CStr;

use bbox::flex::{Bbox, Color as BboxColor};
use log::{debug, warn};
use mdb::{Connection, Subscriber, SubscriberConfig};
use serde::{Deserialize, Serialize};

const TOPIC: &CStr = c"com.axis.consolidated_track.v1.beta";
const SOURCE: &CStr = c"1";

const SENSITIVITY: f64 = 190.0;

#[derive(Debug)]
struct Point2D {
    x: f32,
    y: f32,
}
#[derive(Serialize, Deserialize, Debug)]
struct BoundingBox {
    top: f32,
    left: f32,
    right: f32,
    bottom: f32,
}

impl BoundingBox {
    fn ground_intersection(&self) -> Point2D {
        Point2D {
            x: self.left + (self.right - self.left) / 2.0,
            y: self.bottom,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct Observation {
    bounding_box: BoundingBox,
    timestamp: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct Color {
    name: String,
    score: f32,
}

#[derive(Serialize, Deserialize, Debug)]
struct Class {
    colors: Vec<Color>,
    score: f32,
    #[serde(rename = "type")]
    class_type: ClassType,
}

#[derive(Serialize, Deserialize, Debug)]
enum ClassType {
    Bike,
    Bus,
    Car,
    Human,
    Truck,
    Vehicle,
}
#[derive(Serialize, Deserialize, Debug)]
struct Data {
    #[serde(default = "Vec::new")]
    classes: Vec<Class>,
    duration: f32,
    end_time: Option<String>,
    id: String,
    observations: Vec<Observation>,
    start_time: String,
}

fn main() -> anyhow::Result<()> {
    acap_logging::init_logger();

    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let mut droppable_tx = Some(tx);

    let connection =
        Connection::try_new(Some(Box::new(|e| warn!("Not connected because {e:?}")))).unwrap();
    let config = SubscriberConfig::try_new(
        TOPIC,
        SOURCE,
        Box::new(move |message| {
            let payload = String::from_utf8(message.payload().to_vec());
            let Some(tx) = &droppable_tx else {
                debug!("Dropping message because sender was previously dropped");
                return;
            };
            if tx.try_send(payload).is_err() {
                warn!("Dropping sender because receiver has been deallocated");
                droppable_tx = None;
            }
        }),
    )
    .unwrap();
    let _subscriber = Subscriber::try_new(
        &connection,
        config,
        Box::new(|e| match e {
            None => debug!("Subscribed"),
            Some(e) => warn!("Not subscribed because {e:?}"),
        }),
    )
    .unwrap();

    let mut bbox = Bbox::try_view_new(1)?;
    let gold = BboxColor::from_rgb(0xFF, 0xD7, 0x00);
    let orange = BboxColor::from_rgb(0xFF, 0x8C, 0x00);
    let blue = BboxColor::from_rgb(0x00, 0x00, 0xFF);
    let green = BboxColor::from_rgb(0x32, 0xCD, 0x32);
    let red = BboxColor::from_rgb(0x8B, 0x00, 0x00);
    let gray = BboxColor::from_rgb(0x80, 0x80, 0x80);

    bbox.try_clear()?;
    while let Ok(msg) = rx.recv() {
        let msg = msg?;
        let msg = match serde_json::from_str(&msg) {
            Ok(d) => d,
            Err(e) => {
                debug!("Received {msg:?}");
                warn!("Could not deserialize because {e:?}");
                continue;
            }
        };
        let Data {
            end_time,
            observations,
            classes,
            ..
        } = msg;
        if end_time.is_none() {
            debug!("Track has not ended, skipping.");
            continue;
        }
        let Some(class) = classes.first() else {
            warn!("No classes, skipping");
            continue;
        };

        let color = match class.class_type {
            ClassType::Bike => gold,
            ClassType::Bus => orange,
            ClassType::Car => blue,
            ClassType::Human => green,
            ClassType::Truck => red,
            ClassType::Vehicle => gray,
        };

        // The program sometimes exits because one of the bbox calls fail.
        // Not sure which, why or what to do though.
        bbox.try_color(color)?;
        let step = (observations.len() as f64 / SENSITIVITY).ceil().max(1.0) as usize;
        let mut observations = observations.into_iter().step_by(step);
        if let Some(obs) = observations.next() {
            let Point2D { x, y } = obs.bounding_box.ground_intersection();
            bbox.try_move_to(x, y)?;
        }
        for obs in observations {
            let Point2D { x, y } = obs.bounding_box.ground_intersection();
            // On at least one occasion this failed:
            // Protocol not available (os error 92)
            bbox.try_line_to(x, y)?;
        }
        bbox.try_draw_path()?;
        bbox.try_commit(0)?;
    }
    Ok(())
}
