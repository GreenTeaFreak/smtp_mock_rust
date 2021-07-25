#[macro_use]
extern crate lazy_static;

use std::net::{ TcpListener, TcpStream };
use std::io::{ BufReader, BufRead, Write };
use std::fs::File;
use std::{ thread };
use clap::{ Arg, App, ArgMatches };
use std::collections::HashMap;
use std::fmt;
use rand::Rng;
use std::time::{ SystemTime, UNIX_EPOCH };

lazy_static! {
    static ref COMMANDS: HashMap<&'static str, Command> = {
        let mut map: HashMap<&str, Command> = HashMap::new();

        map.insert("EHLO",          Command::default_plain("EHLO", 500));
        map.insert("HELO",          Command::default_plain("HELO", 250));
        map.insert("MAIL FROM:",    Command::default_plain("MAIL FROM:", 250));
        map.insert("RCPT TO:",      Command::default_plain("RCPT TO:", 250));
        map.insert("HELP",          Command::default_plain("HELP", 502));
        map.insert("NOOP",          Command::default_plain("NOOP", 250));
    
        map.insert("DATA", Command::default("DATA", 354, |_, ctx| { 
            ctx.set_mode(ClientMode::DATA);
        }));
    
        map.insert("RSET", Command::default("RSET", 250, |_, ctx| { 
            ctx.set_mode(ClientMode::INITIAL);
        }));
    
        map.insert("QUIT", Command::default("QUIT", 221, |_, ctx| { 
            ctx.set_mode(ClientMode::FINISHED);
        }));
        
        map
    };

    static ref RESPONSES : HashMap<u16, &'static str> = {
        let mut map = HashMap::new();
        map.insert(211, "211 System status, or system help reply\r\n");
        map.insert(220, "220 Service ready\r\n");
        map.insert(221, "221 Service closing transmission channel\r\n");
        map.insert(250, "250 Requested mail action okay, completed\r\n");
        map.insert(354, "354 Start mail input; end with <CRLF>.<CRLF>\r\n");
        map.insert(500, "500 Syntax error, command unrecognized\r\n");
        map.insert(502, "502 Command not implemented\r\n");
        map
    };
}

type CmdCallback = fn(&str, &mut ClientContext)->();

struct Command {
    cmd: &'static str, 
    response: u16, 
    tarpit: u32,
    callback: Option<CmdCallback>
} 

impl std::fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(cmd={}, response={}, tarpit={})", self.cmd, self.response, self.tarpit)
    }
}

impl Command {
    fn default(cmd: &'static str, response: u16, callback: CmdCallback) -> Command {
        Command { cmd, response, callback: Some(callback), tarpit: 0 }
    }

    fn default_plain(cmd: &'static str, response: u16) -> Command {
        Command { cmd, response, callback: None, tarpit: 0 }
    }
}

enum ClientMode {
    INITIAL,
    DATA,
    FINISHED
}

struct ClientContext {
    mode: ClientMode, 
    out_file: File, 
}

impl ClientContext {
    fn new() -> ClientContext {
        ClientContext {
            mode: ClientMode::INITIAL,
            out_file: ClientContext::open_out_file()
        }
    }

    //FIXME: this is ugly, but I'm lazy...
    fn build_file_name() -> String {
        let r = rand::thread_rng().gen_range(0..100000);

        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("get system time")
            .as_millis();

       let n = format!("MAIL_{}_{}.txt", t, r);
       println!("filename={}", n);
       n
    }

    fn open_out_file() -> File {
        let fname = ClientContext::build_file_name();
        File::create(fname).unwrap()
    }

    fn write_to_file(&mut self, line: String) {
        write!(self.out_file, "{}", line).expect("write to file");
    }

    fn set_mode(&mut self, new_mode: ClientMode) {
        self.mode = new_mode;
    }

    fn flush_out_file(&mut self) {
        //FIXME: how to close this ?!?
        self.out_file.flush().unwrap();
    }
}

//FIXME: add thread pool backed handling
fn bootstrap_client_threaded(stream: TcpStream) {
    thread::spawn(|| { handle_client(stream); });
}

fn build_bind_addr(matches: &ArgMatches) -> String {
    let mut bind_addr = String::with_capacity(24);
    bind_addr.push_str(matches.value_of("bindaddress").unwrap());
    bind_addr.push_str(":");
    bind_addr.push_str(matches.value_of("bindport").unwrap());
    bind_addr
}

fn handle_client(stream: TcpStream) {
    println!("got client connection");

    //FIXME: add read timeout
    // stream.set_read_timeout(Some(Duration::from_secs(5))).expect("set timeout");

    let mut ctx = ClientContext::new();
    let mut xs = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    
    loop {
        let mut line = String::new();
        let res = reader.read_line(&mut line);

        match res {
            Ok(line_count) => { 
                if line_count <= 0 {
                    ctx.flush_out_file(); 
                    return; 
                } 

                match ctx.mode {
                    ClientMode::INITIAL => {
                        let s = line.trim();
                        match COMMANDS.get(s) {
                            Some(cmd) => {
                                let r = RESPONSES.get(&cmd.response).unwrap();
                                if let Some(c) = cmd.callback {
                                    c(s, &mut ctx);
                                }
                                xs.write(r.as_bytes()).unwrap();

                                // "QUIT" ?
                                if cmd.response == 221 {
                                    return;
                                }
                            }
                            None => { 
                                let r = RESPONSES.get(&500).unwrap();
                                xs.write(r.as_bytes()).unwrap();
                            }
                        }
                    }
                    ClientMode::DATA => {
                        let s = line.trim();
                        if s == "." {
                            ctx.set_mode(ClientMode::INITIAL);
                            let r = RESPONSES.get(&250).unwrap();
                            xs.write(r.as_bytes()).unwrap();
                        } 
                        else {
                            ctx.write_to_file(line);
                        }
                    }
                    _ => ()
                }    
            }
            Err(e) => { println!("ERROR={}", e); }
        }
    }
}

fn main() {
    let matches = App::new("smtp_mock_rust")
        .version("0.0.1")
        .arg(
            Arg::with_name("file")
                .short("f")
                .long("file")
                .takes_value(true)
                .default_value("smtp_mock_rust.txt")
                .help("output filename")
        )
        .arg(
            Arg::with_name("bindport")
                .short("p")
                .long("bindport")
                .takes_value(true)
                .default_value("2525")
                .help("bind to this port")
        )
        .arg(
            Arg::with_name("bindaddress")
                .short("a")
                .long("bindaddress")
                .takes_value(true)
                .default_value("127.0.0.1")
                .help("bind to this address")
        )
        .get_matches();

    let bind_addr = build_bind_addr(&matches);
    let out_file = matches.value_of("file").unwrap();
    let listener = TcpListener::bind(&bind_addr).expect("Could not bind");
    
    println!("bound to={}, dumping to file={}\nwaiting for connections...", 
        bind_addr, 
        out_file);

    loop {
        match listener.accept() {
            Ok((stream, addr)) => {
                println!("new client: {:?}", addr);
                bootstrap_client_threaded(stream);
                // handle_client(stream);
            },
            Err(e) => println!("couldn't get client: {:?}", e),
        }
    }
}