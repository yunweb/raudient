
use std::str;
use std::thread;
use model::Packet;
use std::net::UdpSocket;
use std::net::{TcpStream, TcpListener};
use std::sync::mpsc;
use std::collections::HashMap;
use std::time::{self, Duration, SystemTime, UNIX_EPOCH};
use std::io::Read;
use std::io::Write;
use std::fs::{self, File, Metadata};
use std::io::BufReader;

use constant;
use model::{self, User, OperUser, Operate};
use chrono::prelude::*;
use encoding::{Encoding, EncoderTrap, DecoderTrap};
use encoding::all::GB18030;
use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use gtk;
use gtk::TreeModelExt;
use gtk::prelude::*;
use gtk::{
    CellRendererText, AboutDialog, CheckMenuItem, IconSize, Image, Label, Menu, MenuBar, MenuItem, Window,
    WindowPosition, WindowType, StatusIcon, ListStore, TreeView, TreeViewColumn, Builder, Grid, Button, Orientation,
    ReliefStyle, Widget, TextView, Fixed, ScrolledWindow, Alignment,
};
use message;


///启动消息监听线程
pub fn start_daemon(sender: mpsc::Sender<Packet>){
    ::demons::GLOBAL_UDPSOCKET.with(|global| {
        if let Some(ref socket) = *global.borrow() {
            let socket_clone = socket.try_clone().unwrap();
            thread::spawn(move||{
                loop {
                    let mut buf = [0; 2048];
                    match socket_clone.recv_from(&mut buf) {
                        Ok((amt, src)) => {
                            //let receive_str = unsafe { str::from_utf8_unchecked(&buf[0..amt])};
                            //todo 默认是用中文编码 还没想到怎么做兼容
                            let receive_str = GB18030.decode(&buf[0..amt], DecoderTrap::Strict).unwrap();
                            println!("收到原始信息 -> {} 来自 ip -> {}", receive_str, src.ip());
                            let v: Vec<&str> = receive_str.splitn(6, |c| c == ':').collect();
                            if v.len() > 4 {
                                let mut packet = Packet::from(String::from(v[0]),
                                                              String::from(v[1]),
                                                              String::from(v[2]),
                                                              String::from(v[3]),
                                                              v[4].parse::<u32>().unwrap(),
                                                              None
                                );
                                if v.len() > 5 {
                                    packet.additional_section = Some(String::from(v[5]));
                                }
                                packet.ip = src.ip().to_string();
                                sender.send(packet);
                            }else {
                                println!("Invalid packet {} !", receive_str);
                            }
                        },
                        Err(e) => {
                            println!("couldn't recieve a datagram: {}", e);
                        }
                    }
                }
            });
        }
    });
}

///信息处理
pub fn start_message_processer(receiver :mpsc::Receiver<Packet>, sender :mpsc::Sender<OperUser>, remained_sender :mpsc::Sender<((String, String), Option<Packet>)>){
    ::demons::GLOBAL_UDPSOCKET.with(|global| {
        if let Some(ref socket) = *global.borrow() {
            let socket_clone = socket.try_clone().unwrap();
            thread::spawn(move || {
                loop {
                    let packet: Packet = receiver.recv().unwrap();
                    let extstr = packet.clone().additional_section.unwrap_or("".to_owned());
                    let opt = constant::get_opt(packet.command_no);
                    let cmd = constant::get_mode(packet.command_no);
                    println!("{:?}", packet);
                    println!("命令位 {:x} 扩展位 {:x} 扩展段 {}", cmd, opt, extstr);
                    let addr:String = format!("{}:{}", packet.ip, constant::IPMSG_DEFAULT_PORT);
                    if opt&constant::IPMSG_SENDCHECKOPT != 0 {
                        let recvmsg = Packet::new(constant::IPMSG_RECVMSG, Some(packet.packet_no.to_string()));
                        socket_clone.send_to(recvmsg.to_string().as_bytes(), addr.as_str()).expect("couldn't send message");
                    }
                    if cmd == constant::IPMSG_BR_EXIT {//收到下线通知消息
                        let user = User::new(packet.sender_name, packet.sender_host, packet.ip, "".to_owned());
                        sender.send(OperUser::new(user, Operate::REMOVE));
                        ::glib::idle_add(receive);
                    } else if cmd == constant::IPMSG_BR_ENTRY {//收到上线通知消息
                        ///扩展段 用户名|用户组
                        let ext_vec = extstr.splitn(2, |c| c == ':').collect::<Vec<&str>>();
                        let ansentry_packet = Packet::new(constant::IPMSG_ANSENTRY, None);
                        socket_clone.set_broadcast(false).unwrap();
                        socket_clone.send_to(ansentry_packet.to_string().as_bytes(), addr.as_str()).expect("couldn't send message");
                        let group_name = if ext_vec.len() > 2 {
                            ext_vec[1].to_owned()
                        }else {
                            "".to_owned()
                        };
                        let user_name = if ext_vec.len() > 1&& !ext_vec[0].is_empty() {
                            ext_vec[0].to_owned()
                        }else {
                            packet.sender_name
                        };
                        let user = User::new(user_name, packet.sender_host, packet.ip, group_name);
                        sender.send(OperUser::new(user, Operate::ADD));
                        ::glib::idle_add(receive);
                    }else if cmd == constant::IPMSG_SENDMSG {//收到发送的消息
                        //文字消息|文件扩展段
                        let ext_vec = extstr.split('\0').collect::<Vec<&str>>();
                        if opt&constant::IPMSG_SECRETOPT != 0 {//是否是密封消息
                            println!("我是密封消息");
                        }
                        let msg_str = if ext_vec.len() > 0 { ext_vec[0] } else { "" };
                        //文字消息内容|文件扩展
                        if opt&constant::IPMSG_FILEATTACHOPT != 0 {
                            if ext_vec.len() > 1 {
                                let files_str: &str = ext_vec[1];
                                println!("我是带文件附件的 {}", files_str);
                                let files = files_str.split(constant::FILELIST_SEPARATOR).into_iter().filter(|x: &&str| !x.is_empty()).collect::<Vec<&str>>();
                                for file_str in files {
                                    let file_attr = file_str.splitn(6, |c| c == ':').into_iter().filter(|x: &&str| !x.is_empty()).collect::<Vec<&str>>();
                                    if file_attr.len() >= 5 {
                                        //fileID:filename:size:mtime:fileattr[:extend-attr=val1[,val2...][:extend-attr2=...]]:\a[:]fileID...
                                        let file_id = file_attr[0];
                                        let file_name = file_attr[1];
                                        let size = file_attr[2];//大小
                                        let mmtime = file_attr[3];//修改时间
                                        let mmtime_num = i64::from_str_radix(mmtime, 16).unwrap();//时间戳
                                        let file_attr = file_attr[4].parse::<u32>().unwrap();//文件属性
                                        let ntime = NaiveDateTime::from_timestamp(mmtime_num as i64, 0);
                                        println!("{}", ntime.format("%Y-%m-%d %H:%M:%S").to_string());
                                        if file_attr == constant::IPMSG_FILE_REGULAR {
                                            println!("普通文件");
                                        }else if file_attr == constant::IPMSG_FILE_DIR {
                                            println!("文件夹");
                                        }
                                    }
                                }
                            };
                        }
                        let packet_clone = packet.clone();
                        remained_sender.send(((packet.sender_name, packet.ip), Some(packet_clone)));
                        ::glib::idle_add(create_or_open_chat);
                    }else {

                    }
                }
            });
        }
    });
}

pub fn start_file_processer() {
    thread::spawn(move || {
        let tcp_listener: TcpListener = TcpListener::bind(constant::addr.as_str()).unwrap();
        println!("start listening!");
        for stream in tcp_listener.incoming() {
            let base_stream = stream.unwrap().try_clone().unwrap();
            thread::spawn(move || {
                let mut stream_echo = base_stream;
                //println!("from {:?}",stream_echo.peer_addr());
                let mut buf = [0; 2048];
                stream_echo.read(&mut buf[..]).unwrap();
                //stream_echo.write("fdjfd".as_bytes());
                //1:1492660195:IPMSG:192.168.0.94:96:58f82fcd:ecbc60e:0:\u{0}ANDROID
                let tmp_str = GB18030.decode(&buf, DecoderTrap::Strict).unwrap();
                let receive_str = tmp_str.trim_right_matches('\u{0}');
                println!("file_processer receive raw str {:?}", receive_str);
                let v: Vec<&str> = receive_str.splitn(6, |c| c == ':').collect();
                if v.len() > 4 {
                    let mut packet = Packet::from(String::from(v[0]),
                                                  String::from(v[1]),
                                                  String::from(v[2]),
                                                  String::from(v[3]),
                                                  v[4].parse::<u32>().unwrap(),
                                                  None
                    );
                    if v.len() > 5 {
                        let cmd = constant::get_mode(packet.command_no);
                        if cmd & constant::IPMSG_GETFILEDATA != 0 {
                            //文件请求
                            println!("文件请求");
                            let file_attr = v[5].splitn(4, |c| c == ':').into_iter().filter(|x: &&str| !x.is_empty()).collect::<Vec<&str>>();
                            if file_attr.len() >= 3 {
                                let packet_id = file_attr[0];
                                let file_id = file_attr[1];
                                let offset = file_attr[2].parse::<u32>().unwrap();
                            }
                            println!("文件报文解析 {:?}", file_attr)
                        } else if cmd & constant::IPMSG_GETDIRFILES != 0 {
                            //文件夹请求
                            println!("文件夹请求");
                        }
                    }
                } else {
                    println!("Invalid packet {} !", receive_str);
                }
            });
        }
    });
}

pub fn create_or_open_chat() -> ::glib::Continue {
    GLOBAL_WINDOWS.with(|global| {
        if let Some((ref mut map, ref rx)) = *global.borrow_mut() {
            if let Ok(((name, host_ip), packet)) = rx.try_recv() {
                let select_map = map.clone();
                if !host_ip.is_empty(){
                    if let Some(chat_win) = select_map.get(&host_ip) {
                        println!("已经存在了");
                        if let Some(pac) = packet {
                            let additional_section =  pac.additional_section.unwrap();
                            //let v = additional_section.split('\0').into_iter().filter(|x: &&str| !x.is_empty()).collect::<Vec<&str>>();
                            let v = additional_section.split('\0').into_iter().collect::<Vec<&str>>();
                            let (start, mut end) = chat_win.his_view.get_buffer().unwrap().get_bounds();
                            chat_win.his_view.get_buffer().unwrap().insert(&mut end, format!("{}:{}\n", pac.sender_name, v[0]).as_str());
                        }
                    }else {
                        let chat_title = &format!("和{}({})聊天窗口", name, host_ip);
                        let chat_window = Window::new(::gtk::WindowType::Toplevel);
                        chat_window.set_title(chat_title);
                        chat_window.set_border_width(5);
                        chat_window.set_position(::gtk::WindowPosition::Center);
                        chat_window.set_default_size(450, 500);
                        let v_chat_box = gtk::Box::new(::gtk::Orientation::Vertical, 0);
                        let h_button_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                        let button1 = gtk::Button::new_with_label("清空");
                        let button2 = gtk::Button::new_with_label("发送");
                        let button3 = gtk::Button::new_with_label("选择文件");
                        h_button_box.add(&button1);
                        h_button_box.add(&button2);
                        h_button_box.add(&button3);
                        let text_view = gtk::TextView::new();
                        let scroll = gtk::ScrolledWindow::new(None, None);
                        scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
                        scroll.set_min_content_height(350);
                        text_view.set_cursor_visible(false);
                        text_view.set_editable(false);
                        scroll.add(&text_view);
                        if let Some(pac) = packet {
                            let additional_section =  pac.additional_section.unwrap();
                            //let v: Vec<&str> = additional_section.split('\0').into_iter().filter(|x: &&str| !x.is_empty()).collect();
                            let v: Vec<&str> = additional_section.split('\0').into_iter().collect();
                            &text_view.get_buffer().unwrap().set_text(format!("{}:{}\n", name, v[0]).as_str());
                        }
                        let text_view_presend = gtk::TextView::new();
                        let scroll1 = gtk::ScrolledWindow::new(None, None);
                        scroll1.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
                        scroll1.set_margin_top(10);
                        scroll1.set_min_content_height(80);
                        scroll1.add(&text_view_presend);
                        v_chat_box.add(&scroll);
                        v_chat_box.add(&scroll1);
                        v_chat_box.add(&h_button_box);
                        chat_window.add(&v_chat_box);
                        let ip_str_1 = host_ip.clone();
                        let ip_str_2 = host_ip.clone();
                        let ip_str_3 = host_ip.clone();
                        let clone_hist_view_event = text_view.clone();
                        button2.connect_clicked(move|_|{
                            let (start_iter, mut end_iter) = text_view_presend.get_buffer().unwrap().get_bounds();
                            let context :&str = &text_view_presend.get_buffer().unwrap().get_text(&start_iter, &end_iter, false).unwrap();
                            println!("{}", context);
                            message::send_ipmsg(context.to_owned(), Vec::new(), ip_str_1.clone());
                            let (his_start_iter, mut his_end_iter) = clone_hist_view_event.get_buffer().unwrap().get_bounds();
                            &clone_hist_view_event.get_buffer().unwrap().insert(&mut his_end_iter, format!("{}:{}\n", "我", context).as_str());
                            &text_view_presend.get_buffer().unwrap().set_text("");
                        });
                        let chat_window1 = chat_window.clone();
                        button3.connect_clicked(move|_|{
                            let file_chooser = gtk::FileChooserDialog::new(
                                Some("打开文件"), Some(&chat_window1), gtk::FileChooserAction::Open);
                            file_chooser.add_buttons(&[
                                ("打开", gtk::ResponseType::Ok.into()),
                                ("取消", gtk::ResponseType::Cancel.into()),
                            ]);
                            if file_chooser.run() == gtk::ResponseType::Ok.into() {
                                let filename = file_chooser.get_filename().unwrap();
                                let metadata: Metadata = fs::metadata(filename).unwrap();
                                if metadata.is_file() {

                                }else if metadata.is_dir() {

                                };
                                let modify_time: time::SystemTime = metadata.modified().unwrap();
                                let chrono_time = ::util::system_time_to_date_time(modify_time);
                                let local_time = chrono_time.with_timezone(&::chrono::Local);
                                /*let file_info = model::FileInfo {
                                    file_id: Local::now().timestamp() as i32,
                                    file_name: filename,
                                    attr: 1,
                                    size: ,
                                    mtime: (),
                                    atime: (),
                                    crtime: (),
                                    is_selected: (),
                                };*/
                            }
                            file_chooser.destroy();
                        });
                        chat_window.show_all();
                        chat_window.connect_delete_event(move|_, _| {
                            GLOBAL_WINDOWS.with(|global| {
                                if let Some((ref mut map1, _)) = *global.borrow_mut() {
                                    map1.remove(&ip_str_3);
                                }
                            });
                            Inhibit(false)
                        });
                        let clone_chat = chat_window.clone();
                        let clone_hist_view = text_view.clone();
                        map.insert(ip_str_2, ChatWindow{ win: clone_chat, his_view:  clone_hist_view});
                    }
                }
            }
        }
    });
    ::glib::Continue(false)
}

fn receive() -> ::glib::Continue {
    GLOBAL.with(|global| {
        if let Some((ref store, ref rx)) = *global.borrow() {
            if let Ok(op_user) = rx.try_recv() {
                let income_user = op_user.user;
                let oper = op_user.oper;
                if oper == Operate::ADD {
                    let mut in_flag = false;
                    if let Some(first) = store.get_iter_first(){//拿出来第一条
                        let mut num :u32 = store.get_string_from_iter(&first).unwrap().parse::<u32>().unwrap();//序号 会改变
                        let ip = store.get_value(&first, 3).get::<String>().unwrap();//获取ip
                        if ip == income_user.ip {
                            in_flag = true;
                        }else {
                            loop {
                                num = num + 1;
                                if let Some(next_iter) = store.get_iter_from_string(&num.to_string()){
                                    let next_ip = store.get_value(&next_iter, 3).get::<String>().unwrap();//获取ip
                                    if next_ip == income_user.ip {
                                        in_flag = true;
                                        break;
                                    }
                                }else{
                                    break;
                                }
                            }
                        }
                    }
                    if !in_flag {
                        store.insert_with_values(None, &[0, 1, 2, 3], &[&&income_user.name, &&income_user.group, &&income_user.host, &&income_user.ip]);
                    }
                }
                if oper == Operate::REMOVE {
                    if let Some(first) = store.get_iter_first(){//拿出来第一条
                        let mut num :u32 = store.get_string_from_iter(&first).unwrap().parse::<u32>().unwrap();//序号 会改变
                        let ip = store.get_value(&first, 3).get::<String>().unwrap();//获取ip
                        if ip == income_user.ip {
                            store.remove(&first);
                        }else {
                            loop {
                                num = num + 1;
                                if let Some(next_iter) = store.get_iter_from_string(&num.to_string()){
                                    let next_ip = store.get_value(&next_iter, 3).get::<String>().unwrap();//获取ip
                                    if next_ip == income_user.ip {
                                        store.remove(&next_iter);
                                        break;
                                    }
                                }else{
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    });
    ::glib::Continue(false)
}

#[derive(Clone)]
pub struct ChatWindow {
    pub win :Window,
    pub his_view :TextView,
}

thread_local!(
    pub static GLOBAL: RefCell<Option<(::gtk::ListStore, mpsc::Receiver<OperUser>)>> = RefCell::new(None);//UdpSocket
    pub static GLOBAL_UDPSOCKET: RefCell<Option<UdpSocket>> = RefCell::new(None);
    pub static GLOBAL_WINDOWS: RefCell<Option<(HashMap<String, ChatWindow>, mpsc::Receiver<((String, String),Option<Packet>)>)>> = RefCell::new(None);
    //pub static GLOBAL_Test: RefCell<Option<Arc<Mutex<Vec<OperUser>>>>> = RefCell::new(None);
);