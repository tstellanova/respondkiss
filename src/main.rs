use std::env;
use std::fs;
//use core::mem;
//use std::collections::HashSet;
use std::path::Path;
use std::io::{BufWriter, Write};
use notify::{Watcher, RecommendedWatcher, RecursiveMode, Result, Config, event::{EventKind, AccessKind, AccessMode} };
use chrono::{DateTime, Local, TimeDelta, Timelike};
use std::sync::Mutex;
use regex_macro::regex;
use core::sync::atomic::{AtomicI64, Ordering};
use indexmap::IndexSet;

const MY_CALL:&'static str = "KM6NLE";
const MY_CALL_EXT:&'static str = "KM6NLE-6";
const MY_SYMBOL:&'static str = "`"; // dish antenna
const BLN_QSL_BASE:&'static str = "BLNQSL"; // Base bulletin QSL group
//const BLN_QSL_GROUP:&'static str = "BLNQSL1/1"; // Bulletin QSL group
const HEARD_GROUP:&'static str = "BLN0QSLs "; // Bulletin 0 QSL group
static SESSION_START_MS: AtomicI64 = AtomicI64::new(0);
static LAST_BROADCAST_MS: AtomicI64 = AtomicI64::new(0);

static HEARD_LIST: Mutex<Vec<String>> = Mutex::new(vec!());
static ACKED_LIST: Mutex<Vec<String>> = Mutex::new(vec!());

const BROADCAST_TIMEOUT:TimeDelta = TimeDelta::seconds(27);
const SESSION_TIMEOUT:TimeDelta = TimeDelta::minutes(10);

fn handle_in_msg(full_msg: &str, out_path: &Path) -> Result<() > {

  // in message must have "header : body" format messages
  let (msg_header, msg_body) = full_msg.split_once(':').unwrap();
  if !msg_header.contains("RS0ISS") {
    println!("RK ignore_local_rx \n");
    return Ok(())
  } 
  let header_re = regex!(r"\[([0-9]+)\].([a-zA-Z0-9-_]+)>([a-zA-Z0-9-_]+)");
  let addressee_re = regex!(r":([a-zA-Z0-9-\/\s]{9}):");

  // sample messages from KISS inbox:
  // [0] RS0ISS>0P0PS1,APRSAT:'v&l SI]ARISS-International Space Station=
  // [0] K6SQY-7>TU4RWT,RS0ISS*,WIDE2-1:`2Ipl s/`"3r}ISS ops @ sea_0
  // [0] N6RVI-6>APTIGR,RS0ISS*:ARISS Ground Monitoring Station  DM12lr
  // [0] W0SX-10>CQ,RS0ISS*:=3701.15N/11332.30W-Hello from Utah! DM37 w0sx@arrl.org {UISS54}
  // [0] OM4ATC>CQ,RS0ISS*,qAU,DB0FOR-10:=4911.77N/01812.04E-Lysa pod Makytou JN99CE Hello from micronation USK via ISS
  // [0] W0JW-6>CQ,RS0ISS*,qAO,KC9IRH::Heard :KF0BMD-6,KM6NLE-6,KF0BMD-7,WB0YAF,KC5ILO-10,KB9RUG,VE7EPN-7,
  // [0] KA7BZG]CQ,RS0ISS*,qAO,KM6NLE-6::BLNQSL1/1:N6YG,KM6NLE-6{UISS54}

  // TODO simplify this to just grab the first header info
  let mut results = vec![];
  for (_, [chan_num, origin, dest ] ) in header_re.captures_iter(&msg_header).map(|c| c.extract()) {
      results.push(( chan_num, origin, dest ));
  }
  let split_header_opt = results.get(0);

  let mut addressee_opt = None;
  if let Some(caps) = addressee_re.captures(&msg_body) {
    if let Some(first_cap) = caps.get(1) {
      addressee_opt = Some(first_cap.as_str());
      //println!("addressee_opt: {:?}", addressee_opt);
    }
  }

  let now = Local::now();
  let timestamp_str = format!("{:02}:{:02}:{:02}", now.hour(), now.minute(), now.second());
  
  let session_start_opt = {
    let prior_ms = SESSION_START_MS.load(Ordering::Relaxed);
    if let Some(prior_dt) = DateTime::from_timestamp_millis(prior_ms) {
      let elapsed = now.signed_duration_since(prior_dt);
      if elapsed > SESSION_TIMEOUT { 
        println!("RK: clear session data, elapsed: {}", elapsed);
        *HEARD_LIST.lock().unwrap() = vec!();
        *ACKED_LIST.lock().unwrap() = vec!();
        let now_ms = now.timestamp_millis();
        SESSION_START_MS.store(now_ms, Ordering::Relaxed);
        None
      }
      else { Some(prior_dt) } 
    }
    else { None } 
  };

  // retrieve list of all call signs heard during this session
  let mut heard_set = {
    HEARD_LIST.lock().expect("failed")
    .clone()
    .into_iter()
    .collect::<IndexSet<_>>()
  };
  println!("RK heard_set.len: {}", heard_set.len());

  // retrieve list of call signs acked during this session
  let mut ack_set = {
    ACKED_LIST.lock().expect("failed")
    .clone()
    .into_iter()
    .collect::<IndexSet<_>>()
  };
  println!("RK ack_set.len: {}", ack_set.len());

  // expire the last broadcast time if we've been idle too long
  let broadcast_time_opt = {
    let prior_ms = LAST_BROADCAST_MS.load(Ordering::Relaxed);
    if let Some(broadcast_time) = DateTime::from_timestamp_millis(prior_ms) {
      let elapsed = now.signed_duration_since(broadcast_time);
      if elapsed > BROADCAST_TIMEOUT { None } 
      else { Some(broadcast_time) }
    }
    else { None }
  };


  if let Some(in_header) = split_header_opt {
    let origin = in_header.1;
    let origin_str =  origin.to_string();
    let dest = in_header.2;

    if !origin.eq(MY_CALL_EXT) && !origin.eq("RS0ISS") {
      heard_set.insert(origin_str.clone());
    }
 
    let mut response_opt =
      if dest.contains(MY_CALL) {
        let new_ack = ack_set.insert(origin_str.clone());        
        if new_ack { Some(format!("{}>{},ARISS:AFK QSL 73 CM87uu {}",MY_CALL_EXT, origin, timestamp_str)) }
        else { None } 
      }
      else if origin.eq(MY_CALL_EXT) {
        // don't respond to our own (repeated) messages
        println!("RK: ignore_self");
        None
      }
      else if origin.eq("RS0ISS") && dest.starts_with("0P0PS") {
        // ARISS beacon sent when there's been no received packets for a while (as little as 3 seconds, every 3 minutes)
	// [0] RS0ISS>0P0PS1,APRSAT:'v&l SI]ARISS-International Space Station=
	// Send location beacon to ARISS at least once per session
        if session_start_opt.is_none() {
          let now_ms = now.timestamp_millis();
          SESSION_START_MS.store(now_ms, Ordering::Relaxed);
          // Send location beacon to ARISS
          Some(format!("{}>CQ,ARISS:=3752.42N/12217.42W{} CQ via ARISS {}",
                                  MY_CALL_EXT, MY_SYMBOL, timestamp_str))
        }
        else { None } 
      }
      else if dest.eq("CQ") || dest.starts_with("AP") { // Catch all APRS clients -- sorry, Pakistan
        // is there a specific addressee?
        if let Some(addressee) = addressee_opt {
          if addressee.contains(&MY_CALL_EXT) {
            let new_ack = ack_set.insert(origin_str.clone());
            if new_ack { Some(format!("{}>CQ,ARISS::{:<9}:AFK QSL 73 CM87uu {}",MY_CALL_EXT, origin, timestamp_str)) }
            else { None }
          }
          else if addressee.contains("CQ")  {
            if !ack_set.contains(&origin_str) {
              Some(format!("{}>CQ,ARISS::{:<9}:QSL 73 CM87 {}",MY_CALL_EXT, origin, timestamp_str)) 
            }
            else { None }
          }
          else if addressee.contains(&BLN_QSL_BASE) || addressee.contains(&HEARD_GROUP) || addressee.contains("Heard") {
            println!("RK Heard/BLNSQL");
            if msg_body.contains(MY_CALL) {
              let new_ack = ack_set.insert(origin_str.clone());
              if new_ack { Some(format!("{}>CQ,ARISS::{:<9}:TNX 73 CM87uu {}",MY_CALL_EXT, origin, timestamp_str)) }
              else { None }
            }
            else { None } 
          }
          else {
            // there's an addressee who's not us: Don't respond
            println!("RK: ignore_notme {}", addressee);
	    None
          }  
        }
        else {
          // no addressee....does the body contain our call sign?
          if msg_body.contains(MY_CALL) || msg_body.contains("cq") || msg_body.contains("CQ") {
            let new_ack = ack_set.insert(origin_str.clone());
            if new_ack { Some(format!("{}>CQ,ARISS::{:<9}:QSL 73 CM87 {}",MY_CALL_EXT, origin, timestamp_str)) }
            else { None } 
          }
          else { None }
        }
      }
      else {
        //println!("RK wacky src: {:?} dst: {:?}", origin, dest);
	None
      };

    if response_opt.is_none() && broadcast_time_opt.is_none() {
      // we've been idle too long -- send something
      println!("RK: idle beacon!");
      if heard_set.len() > 1 {
        // ack the heard list
        let heards = heard_set.iter().map(|x| x.to_string() + ",").collect::<String>();
        response_opt = Some(format!("{}>CQ,ARISS::{}:{}", MY_CALL_EXT, HEARD_GROUP, heards ));
        if heard_set.len() > 4 {
          println!("RK: clear heard_set");
          heard_set.clear();
        } 
      }
      else {
        // Send location beacon to ARISS
        response_opt = Some(format!("{}>CQ,ARISS:=3752.42N/12217.42W{} CQ via ARISS {}",
                                  MY_CALL_EXT, MY_SYMBOL, timestamp_str));
        let now_ms = now.timestamp_millis();
        SESSION_START_MS.store(now_ms, Ordering::Relaxed);
      } 
    }

    // stash the heard list for the next wakeup
    if !heard_set.is_empty() {
      let heard_out:  Vec<_>  = heard_set.iter().map(String::from).collect();
      println!("RK heard_out.len(): {}", heard_out.len());
      *HEARD_LIST.lock().unwrap() = heard_out;
    }
    else {
      *HEARD_LIST.lock().unwrap() = vec!();
    }

    // stash the acked list for the next wakeup
    if !ack_set.is_empty() {
      let ack_out: Vec<_> = ack_set.iter().map(String::from).collect();
      println!("RK ack_out.len(): {}", ack_out.len());
      *ACKED_LIST.lock().unwrap() = ack_out;
    }

    if response_opt.is_some() {
      let now_ms = now.timestamp_millis();
      LAST_BROADCAST_MS.store(now_ms, Ordering::Relaxed);
    }
    else {
      return Ok(())
    } 

    let resp_msg = response_opt.unwrap();
    let now_str = now.to_rfc3339();
    println!("RK {} out:\n{}\n", now_str, resp_msg);

    let full_out_path = out_path.join(now_str);
    let out_file = fs::File::create(full_out_path)?;
    let mut writer = BufWriter::new(out_file);
    writer.write_all(resp_msg.as_bytes())?;
    writer.flush()?;
    //println!("wrote to: {}", now_str);
  }

  Ok(())
}

fn monitor_msgs<P: AsRef<Path> >(in_path: P, out_path:&Path, _save_path:&Path ) -> notify::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();

    // Automatically select the best implementation for your platform.
    // You can also access each implementation directly e.g. INotifyWatcher.
    let mut watcher = RecommendedWatcher::new(tx, Config::default())?;

    // Add a path to be watched. All files and directories at that path and
    // below will be monitored for changes.
    watcher.watch(in_path.as_ref(), RecursiveMode::Recursive)?;

    for res in rx {
        match res {
            Ok(event) =>  {
              // Watch for files that are closed after writing (indicating kissutil is done writing)
              if event.kind == EventKind::Access(AccessKind::Close(AccessMode::Write)) {
                let in_msg_path = event.paths[0].clone();
                if let Ok(msg_body) = fs::read_to_string(&in_msg_path) {
                  let now_str = Local::now().to_rfc3339();
                  println!("RK {}  in:\n{}", now_str, msg_body);
                  handle_in_msg(&msg_body, &out_path)?;
                }
                else {
                  println!("failed to read: {:?}", in_msg_path);
                }
                fs::remove_file(&in_msg_path)?;
              } 
            },
            Err(e) => println!("RK watch error: {:?}", e),
        }
    }

    Ok(())
}


fn main() -> Result< ()>  {
  let args: Vec<String> = env::args().collect();

  println!("RK {} start! with:\r\n {:?}", Local::now(), args);
  if args.len() < 4 {
   println!("RK usage: respondkiss in_path out_path save_path ");
   return Ok(());
  }
  
  let in_path = Path::new(&args[1]);
  let out_path = Path::new(&args[2]); 
  let save_path = Path::new(&args[3]);
  monitor_msgs(in_path, out_path, save_path )?;

  Ok(())
}
