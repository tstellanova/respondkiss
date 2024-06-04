use std::env;
use std::fs;
use core::mem;
use std::collections::HashSet;
use std::path::Path;
use std::io::{BufWriter, Write};
use notify::{Watcher, RecommendedWatcher, RecursiveMode, Result, Config, event::{EventKind, AccessKind, AccessMode} };
use chrono::{DateTime, Local, TimeDelta, Timelike};
use std::sync::Mutex;
use regex_macro::regex;
//use regex::Regex;
use core::sync::atomic::{AtomicI64, Ordering};

const MY_CALL:&'static str= "KM6NLE";
const MY_CALL_EXT:&'static str= "KM6NLE-6";
const MY_SYMBOL:&'static str="`"; // dish antenna
//const HEARD_PREAMBLE:&'static str = "Heard";

static SESSION_START_MS: AtomicI64 = AtomicI64::new(0);
static LAST_BROADCAST_MS: AtomicI64 = AtomicI64::new(0);

//static SESSION_START: Mutex<Option<DateTime<Local>>> = Mutex::new(None);
//static LAST_BROADCAST: Mutex<Option<DateTime<Local>>> = Mutex::new(None);
static SESSION_HERD: Mutex<Option<Vec<String>>> = Mutex::new(None);

const BROADCAST_TIMEOUT:TimeDelta = TimeDelta::seconds(33);
const SESSION_TIMEOUT:TimeDelta = TimeDelta::minutes(10);

fn handle_in_msg(full_msg: &str, out_path: &Path) -> Result<() > {

  // in message must have "header : body" format messages
  let (msg_header, msg_body) = full_msg.split_once(':').unwrap();

  let header_re = regex!(r"\[([0-9]+)\].([a-zA-Z0-9-_]+)>([a-zA-Z0-9-_]+)");
  let addressee_re = regex!(r":([ a-zA-Z0-9-\s]{9}):");

  // sample messages from KISS inbox:
  // [0] RS0ISS>0P0PS1,APRSAT:'v&l SI]ARISS-International Space Station=
  // [0] K6SQY-7>TU4RWT,RS0ISS*,WIDE2-1:`2Ipl s/`"3r}ISS ops @ sea_0
  // [0] N6RVI-6>APTIGR,RS0ISS*:ARISS Ground Monitoring Station  DM12lr
  // [0] W0SX-10>CQ,RS0ISS*:=3701.15N/11332.30W-Hello from Utah! DM37 w0sx@arrl.org {UISS54}
  // [0] OM4ATC>CQ,RS0ISS*,qAU,DB0FOR-10:=4911.77N/01812.04E-Lysa pod Makytou JN99CE Hello from micronation USK via ISS
  // [0] W0JW-6>CQ,RS0ISS*,qAO,KC9IRH::Heard :KF0BMD-6,KM6NLE-6,KF0BMD-7,WB0YAF,KC5ILO-10,KB9RUG,VE7EPN-7,

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
    if prior_ms != 0 {
      if let Some(prior_dt) = DateTime::from_timestamp_millis(prior_ms) {
        let elapsed = now.signed_duration_since(prior_dt);
        if elapsed > SESSION_TIMEOUT { 
          //clear session data 
          *SESSION_HERD.lock().unwrap() = None;
          SESSION_START_MS.store(0, Ordering::Relaxed);  
          None
        }
        else { 
          Some(prior_dt)  
        } 
      }
      else { None } 
    }
    else { None }
  };

  
  /*
  let mut session_start_opt = *SESSION_START.lock().unwrap();
  if let Some(start_time) =  session_start_opt  {
   let elapsed = now.signed_duration_since(start_time);
   if elapsed > SESSION_TIMEOUT {
     session_start_opt = None;
     //clear session data
     *SESSION_HERD.lock().unwrap() = None;
     *SESSION_START.lock().unwrap() = None;
   }
  }
  */

  // retrieve list of call signs heard during this session
  let sess_herd_list = {
    let mut state = SESSION_HERD.lock().expect("failed");
    let new_state = mem::replace(&mut *state, None);
    if new_state.is_none() { Vec::new() }
    else { new_state.unwrap() }
  };
  let mut herd_set = HashSet::<String>::from_iter(sess_herd_list);

  // expire the last broadcast time if we've been idle too long

  let broadcast_time_opt = {
    let prior_ms = LAST_BROADCAST_MS.load(Ordering::Relaxed);
    if prior_ms != 0 {
      if let Some(broadcast_time) = DateTime::from_timestamp_millis(prior_ms) {
        let elapsed = now.signed_duration_since(broadcast_time);
        if elapsed > BROADCAST_TIMEOUT { None } 
        else { Some(broadcast_time) }
      }
      else { None }
    }
    else { None }
  };

  /*
  let mut broadcast_time_opt = *LAST_BROADCAST.lock().unwrap();
  if let Some(broadcast_time) = broadcast_time_opt {
    let elapsed = now.signed_duration_since(broadcast_time);
    if elapsed > BROADCAST_TIMEOUT {
      broadcast_time_opt = None;
    }
   }
  */

  if let Some(in_header) = split_header_opt {
    let origin = in_header.1;
    let dest = in_header.2;

    if !origin.eq(MY_CALL_EXT) && !origin.eq("RS0ISS") {
        herd_set.insert(origin.to_string());
    }
 
    let mut response_opt =
      if dest.contains(MY_CALL) {
        //Some(format!("{}>{},ARISS:=3752.42N/12217.42W{} QSL 73s", MY_CALL_EXT, origin,MY_SYMBOL))
        Some(format!("{}>{},ARISS:{} AFK QSO 73s CM87uu {}",MY_CALL_EXT, origin, MY_SYMBOL,  timestamp_str))
      }
      else if origin.eq(MY_CALL_EXT) {
        // don't respond to our own (repeated) messages
        println!("RK: ignoring message from self");
        None
      }
      else if origin.eq("RS0ISS") && dest.starts_with("0P0PS") {
	// [0] RS0ISS>0P0PS1,APRSAT:'v&l SI]ARISS-International Space Station=
        // ARISS beacon sent when there's been no received packets for a while
	// Send location beacon to ARISS at least once per session
        Some(format!("{}>CQ,ARISS:=3752.42N/12217.42W{} CQ via ARISS {}",
                                MY_CALL_EXT, MY_SYMBOL, timestamp_str))
      }
      else if dest.eq("CQ") || dest.starts_with("AP") { // Catch all APRS clients -- sorry, Pakistan
        // is there a specific addressee?
        if let Some(addressee) = addressee_opt {
          if addressee.contains(&MY_CALL_EXT) || addressee.contains("CQ")  {
            Some(format!("{}>CQ,ARISS::{:<9}: AFK QSL 73s CM87uu {}",MY_CALL_EXT, origin, timestamp_str))
          }
          else {
            // there's an addressee who's not us: Don't respond
            println!("RK: ignore addressee {}", addressee);
	    None
          }  
        }
        else {
          // no addressee....does the body contain our call sign?
          if msg_body.contains(MY_CALL) {
            Some(format!("{}>CQ,ARISS::{:<9}: QSL 73s {}",MY_CALL_EXT, origin, timestamp_str))
          }
          else {
            // acknowledge heards directly
            //Some(format!("{}>CQ,ARISS::{:<9}: Heard CM87uu {}", MY_CALL_EXT, origin, timestamp_str))
            None
          }
        }
      }
      else {
        println!("RK wacky src: {:?} dst: {:?}", origin, dest);
	None
      };

    if response_opt.is_none() && broadcast_time_opt.is_none() {
      // we've been idle too long -- send something
      println!("RK: idle too long -- beacon!");
      if herd_set.is_empty() {
        // Send location beacon to ARISS
        response_opt = Some(format!("{}>CQ,ARISS:=3752.42N/12217.42W{} CQ via ARISS {}",
                                  MY_CALL_EXT, MY_SYMBOL, timestamp_str));
      }
      else {
        // ack the heard list
        let heards = herd_set.iter().map(|x| x.to_string() + ",").collect::<String>();
        response_opt = Some(format!("{}>CQ,ARISS::Heard    :{}", MY_CALL_EXT, heards ));
        if heards.len() > 14 {
          herd_set.clear();
        }
      }
    }

    if !herd_set.is_empty() {
      let herd_out  = herd_set.iter().map(String::from).collect();
      *SESSION_HERD.lock().unwrap() = Some(herd_out);
    }

    if response_opt.is_some() {
      let now_ms = now.timestamp_millis();
      if session_start_opt.is_none() {
        SESSION_START_MS.store(now_ms, Ordering::Relaxed);
	//*SESSION_START.lock().unwrap() = Some(now);
      }   

      LAST_BROADCAST_MS.store(now_ms, Ordering::Relaxed);
      //*LAST_BROADCAST.lock().unwrap() = Some(now);
    }
    else {
      return Ok(())
    } 

    let resp_msg = response_opt.unwrap();
    let now_str = now.to_rfc3339();
    println!("RK {} out: {}", now_str, resp_msg);

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
                  println!("RK {:?} in: {:?}", now_str, msg_body);
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
