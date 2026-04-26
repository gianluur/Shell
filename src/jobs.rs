use crate::{parser::Command, terminal::Terminal};
use anyhow::Result;
use std::{collections::HashMap, fmt, os::fd::RawFd};

pub enum JobState {
    Running,
    Stopped,
}

impl fmt::Display for JobState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = match self {
            JobState::Running => "Running",
            JobState::Stopped => "Stopped",
        };
        write!(f, "{}", state)
    }
}

pub struct Job {
    pub pgid: libc::pid_t,
    pub pids: Vec<libc::pid_t>,
    pub command: Command,
    pub state: JobState,
    pub stdout_fd: Option<RawFd>, // Some for background, None for foreground
    pub remaining: usize,
}

impl Job {
    pub fn new(
        pgid: libc::pid_t,
        pids: Vec<libc::pid_t>,
        command: Command,
        state: JobState,
        stdout_fd: Option<RawFd>,
    ) -> Self {
        Job {
            pgid,
            pids: pids.clone(),
            command,
            state,
            stdout_fd,
            remaining: pids.len(),
        }
    }

    pub fn to_string(&self) -> String {
        format!(
            "PGID: {} | Command: {} | State: {}",
            self.pgid, self.command, self.state,
        )
    }
}

pub struct Jobs {
    pub table: HashMap<usize, Job>,
    pub pgid_to_id: HashMap<libc::pid_t, usize>,
    pub pid_to_id: HashMap<libc::pid_t, usize>,
    pub next_job_id: usize,
}

impl Jobs {
    pub fn new() -> Self {
        Self {
            table: HashMap::new(),
            pgid_to_id: HashMap::new(),
            pid_to_id: HashMap::new(),
            next_job_id: 1,
        }
    }

    pub fn add(&mut self, job: Job) -> usize {
        let id = self.next_job_id;
        self.pgid_to_id.insert(job.pgid, id);
        for &pid in &job.pids {
            self.pid_to_id.insert(pid, id);
        }
        self.table.insert(id, job);
        self.next_job_id += 1;
        id
    }

    pub fn remove(&mut self, id: usize) {
        if let Some(job) = self.table.remove(&id) {
            self.pgid_to_id.remove(&job.pgid);
            for &pid in &job.pids {
                self.pid_to_id.remove(&pid);
            }
            if let Some(fd) = job.stdout_fd {
                unsafe {
                    libc::close(fd);
                }
            }
            if self.table.is_empty() {
                self.next_job_id = 1;
            }
        }
    }

    pub fn get_entry(&mut self, pgid: libc::pid_t) -> Option<(usize, &mut Job)> {
        let &id = self.pgid_to_id.get(&pgid)?;
        let job = self.table.get_mut(&id)?;
        Some((id, job))
    }

    pub fn get_entry_by_pid(&mut self, pid: libc::pid_t) -> Option<(usize, &mut Job)> {
        let &id = self.pid_to_id.get(&pid)?;
        let job = self.table.get_mut(&id)?;
        Some((id, job))
    }

    pub fn update_table(&mut self, terminal: &mut Terminal) -> Result<()> {
        unsafe {
            loop {
                let mut status = 0;

                // For understanding this syscall refer here:
                // https://man7.org/linux/man-pages/man3/wait.3p.html
                // I would literlly just copy and paste the content otherwise
                let pid = libc::waitpid(
                    -1,
                    &mut status,
                    libc::WNOHANG | libc::WUNTRACED | libc::WCONTINUED,
                );

                if pid <= 0 {
                    break;
                }

                if let Some((id, job)) = self.get_entry_by_pid(pid) {
                    let mut notification: Option<String> = None;
                    if libc::WIFEXITED(status) || libc::WIFSIGNALED(status) {
                        job.remaining -= 1;
                        if job.remaining == 0 {
                            notification = Some(format!("[{}] Done      {}", id, job.command));
                            self.remove(id);
                        }
                    } else if libc::WIFSTOPPED(status) {
                        job.state = JobState::Stopped;
                        notification = Some(format!("[{}] Stopped  {}", id, job.command));
                    } else if libc::WIFCONTINUED(status) {
                        job.state = JobState::Running;
                        notification = Some(format!("[{}] Continued", id));
                    }

                    if notification.is_some() {
                        terminal.notifications.push(notification.unwrap());
                    }
                }
            }
        }
        Ok(())
    }

    pub fn get_bg_job_stdout(&mut self) -> Result<Vec<String>> {
        let mut stdout = Vec::new();
        let mut buf = [0u8; 4096];

        for job in self.table.values() {
            if let Some(fd) = job.stdout_fd {
                unsafe {
                    // Set non-blocking
                    let flags = libc::fcntl(fd, libc::F_GETFL);
                    libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);

                    loop {
                        let n = libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len());

                        if n < 0 {
                            let err = std::io::Error::last_os_error();
                            // Break only if it's truly non-blocking, otherwise handle error
                            if err.kind() == std::io::ErrorKind::WouldBlock {
                                break;
                            }
                            return Err(err.into());
                        } else if n == 0 {
                            break; // EOF1
                        }

                        let output = String::from_utf8_lossy(&buf[..n as usize]).into_owned();
                        stdout.push(output);
                    }
                }
            }
        }

        Ok(stdout)
    }

    pub fn wait_foreground(
        &mut self,
        shell_gpid: libc::pid_t,
        terminal: &mut Terminal,
        pgid: libc::pid_t,
        command: Command,
        pids: &[libc::pid_t],
        is_new_job: bool,
    ) -> Result<i32> {
        let mut exit_code = 0;
        let mut stopped = false;
        loop {
            let mut status: libc::c_int = 0;

            // We use -pgid because as we can read in the docs:
            // "If pid is less than (pid_t)-1, status is requested for any
            // child process whose process group ID is equal to the absolute
            // value of pid."
            let pid = unsafe { libc::waitpid(-pgid, &mut status, libc::WUNTRACED) };

            if pid <= 0 {
                break;
            }

            if libc::WIFEXITED(status) {
                if !pids.is_empty() && *pids.last().unwrap() == pid {
                    exit_code = libc::WEXITSTATUS(status);
                }
            } else if libc::WIFSIGNALED(status) {
                if !pids.is_empty() && *pids.last().unwrap() == pid {
                    exit_code = 128 + libc::WTERMSIG(status);
                }
            } else if libc::WIFSTOPPED(status) {
                stopped = true;
            }
        }

        if stopped {
            let id;

            if is_new_job {
                let job = Job::new(pgid, pids.to_vec(), command, JobState::Stopped, None);
                id = self.add(job);
            } else {
                let (job_id, job) = self.get_entry(pgid).unwrap();
                job.state = JobState::Stopped;
                id = job_id;
            }

            terminal.println(&format!("\r\n[{}] Stopped {}", id, pgid))?;
            exit_code = 148;
        } else if !is_new_job {
            if let Some((id, _)) = self.get_entry(pgid) {
                self.remove(id);
            }
        }

        unsafe { libc::tcsetpgrp(libc::STDIN_FILENO, shell_gpid) };

        Ok(exit_code)
    }
}
