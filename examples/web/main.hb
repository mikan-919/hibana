use './syntax' -> *
use './lib'
    -> { hanabi }
    -> { Response, Clock }

fn main() {
    let app = hanabi(
        route {
            GET '/' {
                Response::text('Hello!')
            }
            GET '/status' {
                Response::json({
                    status: 'OK'
                    code: 200
                })
            }
            @durable_object('ACCOUNT') -> account
            GET `/{id}/status?time={time: 'day' | 'night'}` {
                let id = id.get().unwrap()
                let account = account.get(id)
                account.stamp(Clock::now())
                Response::json({
                    status: 'OK'
                    code: 200
                    message: `{if time.get().unwrap() == 'day' {'Hello'} else {'Good evening'}}! {id}`
                })
            }
        }
    )
}
