use regex::Regex;
use std::env;
use std::fs;
use std::path::Path;
use std::io::{BufWriter, Write};
use notify::{Watcher, RecommendedWatcher, RecursiveMode, Result, Config, event::{EventKind, AccessKind, AccessMode} };
use chrono::{Local};

fn handle_msgs(hay: &str, out_path: &Path) -> Result<() > {
  //let re = Regex::new(r"\[([0-9]+)\].([a-zA-Z0-9-_]+)>([a-zA-Z0-9-_]+),([a-zA-Z0-9-_\*]+):").unwrap();
  let re = Regex::new(r"\[([0-9]+)\].([a-zA-Z0-9-_]+)>([a-zA-Z0-9-_]+)").unwrap();


  let mut results = vec![];
  for (_, [chan_num, origin, dest ] ) in re.captures_iter(&hay).map(|c| c.extract()) {
      results.push(( chan_num, origin, dest ));
  }

  println!("Message Count: {}", results.len());
  for result in results {
    //println!("{:?}",result);
    let new_dest = result.1;
    // skip replying to ISS
    if new_dest.contains("ISS") {
      continue;
    }

    // [0] N6RVI-6>APTIGR,RS0ISS*:ARISS Ground Monitoring Station  DM12lr
    // [0] W0SX-10>CQ,RS0ISS*:=3701.15N/11332.30W-Hello from Utah! DM37 w0sx@arrl.org {UISS54}
    let new_msg = format!("KM6NLE-10>{},ARISS:=3752.42N/12217.42W-Roger CM87uu",new_dest);
    println!("out: {}", new_msg);
    let now = Local::now();
    let now_str = now.to_rfc3339();
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

fn monitor_msgs<P: AsRef<Path> >(in_path: P, out_path:&Path ) -> notify::Result<()> {
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
                println!("in: {:?}", hay);
                handle_msgs(&hay, &out_path)?;
                fs::remove_file(&in_msg_path)?;
              } 
            },
            Err(e) => println!("watch error: {:?}", e),
        }
    }

    Ok(())
}


fn main() -> Result< ()>  {
  let args: Vec<String> = env::args().collect();

  println!("start! with: {:?}", args);
  if args.len() < 3 {
   println!("usage: respondkiss in_path out_path ");
   return Ok(());
  }
  
  let in_path = Path::new(&args[1]);
  let out_path = Path::new(&args[2]); 
  monitor_msgs(in_path, out_path )?;

  Ok(())
}
