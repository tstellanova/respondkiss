use regex::Regex;
use std::env;
use std::fs;
use std::path::Path;
use std::io::{BufWriter, Write};
use notify::{Watcher, RecommendedWatcher, RecursiveMode, Result, Config, event::{EventKind, AccessKind, AccessMode} };
use chrono::{DateTime, Local, TimeDelta, Timelike};
use std::sync::Mutex;

const MY_CALL:&'static str= "KM6NLE";
const MY_CALL_EXT:&'static str= "KM6NLE-6";

static LAST_BROADCAST: Mutex<Option<DateTime<Local>>> = Mutex::new(None);

const BROADCAST_TIMEOUT:TimeDelta= TimeDelta::seconds(90);  

fn handle_msgs(hay: &str, out_path: &Path) -> Result<() > {

  //let re = Regex::new(r"\[([0-9]+)\].([a-zA-Z0-9-_]+)>([a-zA-Z0-9-_]+),([a-zA-Z0-9-_\*]+):").unwrap();
  let re = Regex::new(r"\[([0-9]+)\].([a-zA-Z0-9-_]+)>([a-zA-Z0-9-_]+)").unwrap();
  let addressee_re = Regex::new(r"::([ a-zA-Z0-9-\s]{9}):").unwrap();

  // sample messages from KISS inbox:
  // [0] N6RVI-6>APTIGR,RS0ISS*:ARISS Ground Monitoring Station  DM12lr
  // [0] W0SX-10>CQ,RS0ISS*:=3701.15N/11332.30W-Hello from Utah! DM37 w0sx@arrl.org {UISS54}

  let mut results = vec![];
  for (_, [chan_num, origin, dest ] ) in re.captures_iter(&hay).map(|c| c.extract()) {
      results.push(( chan_num, origin, dest ));
  }
  let first_msg = results.get(0); 

  let mut addr_results = vec![];
  for (_, [whole_addressee, inner_addressee ] ) in addressee_re.captures_iter(&hay).map(|c| c.extract()) {
      addr_results.push(( whole_addressee, inner_addressee));
  }
  let first_addressee = addr_results.get(0);
  let now = Local::now();


  let mut broadcast_time_opt = *LAST_BROADCAST.lock().unwrap();
  if let Some(broadcast_time) = broadcast_time_opt {
    let elapsed = now.signed_duration_since(broadcast_time);
    if elapsed > BROADCAST_TIMEOUT {
      broadcast_time_opt = None;
    }
   }

  if let Some(in_header) = first_msg {
    let origin = in_header.1;
    let dest = in_header.2;
 
    let response =  
      if dest.contains(MY_CALL) {
        Some(format!("{}>{},ARISS:=3752.42N/12217.42W(QSL 73s", MY_CALL_EXT, origin))
      }
      else if dest.eq("ALL") {
        Some(format!("{}>{},ARISS::{:<9}: Heard CM87uu",MY_CALL_EXT, origin, origin))
      }
      else if dest.eq("CQ") ||
	dest.starts_with("AP") { // Catch all APRS clients -- sorry, Pakistan
        if let Some(addressee) = first_addressee {
	  println!("RK addressee: {:?}", addressee);
          if addressee.1.contains(&MY_CALL) {
            Some(format!("{}>CQ,ARISS::{:<9}: QSL 73s ",MY_CALL_EXT, origin))
          }
          else {
            // destination is CQ but theres an addressee who's not us
            // Don't respond
	    None
          }  
        }
        else {
          // acknowledge heard 
          Some(format!("{}>CQ,ARISS::{:<9}: Heard CM87uu",MY_CALL_EXT, origin))
        }
      }
      else {
        if broadcast_time_opt.is_none() {
          // track the last 
          broadcast_time_opt = Some(now);
          Some(format!("{}>CQ,ARISS:=3752.42N/12217.42W(Hello ARISS {:02}:{:02}:{:02}", 
		MY_CALL_EXT, now.hour(), now.minute(), now.second()))
        }
        else {
          None
	}
      };

    *LAST_BROADCAST.lock().unwrap() = broadcast_time_opt;

    if response.is_none() {
      return Ok(())
    } 
    let new_msg = response.unwrap();
    let now_str = now.to_rfc3339();
    println!("RK {} out: {}", now_str, new_msg);

    let full_out_path = out_path.join(now_str);
    //println!("full_out_path: {:?}",full_out_path);
    let out_file = fs::File::create(full_out_path)?;
    let mut writer = BufWriter::new(out_file);
    writer.write_all(new_msg.as_bytes())?;
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
              //println!("changed: {:?}", event);
              // kind: Access(Close(Write))
              //println!("{:?}", event.kind);
              if event.kind == EventKind::Access(AccessKind::Close(AccessMode::Write)) {
                //println!("{:?}",event.paths[0]);
                let in_msg_path = event.paths[0].clone();
                let hay = fs::read_to_string(&in_msg_path)?;
                let now = Local::now();
    		let now_str = now.to_rfc3339();
                println!("RK {} in: {:?}", now_str, hay);
                handle_msgs(&hay, &out_path)?;
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
