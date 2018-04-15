(function() {var implementors = {};
implementors["bytes"] = ["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/nightly/core/marker/trait.Sync.html\" title=\"trait core::marker::Sync\">Sync</a> for <a class=\"struct\" href=\"bytes/alloc/struct.MemRef.html\" title=\"struct bytes::alloc::MemRef\">MemRef</a>","impl <a class=\"trait\" href=\"https://doc.rust-lang.org/nightly/core/marker/trait.Sync.html\" title=\"trait core::marker::Sync\">Sync</a> for <a class=\"struct\" href=\"bytes/struct.Bytes.html\" title=\"struct bytes::Bytes\">Bytes</a>",];
implementors["mio"] = ["impl&lt;H:&nbsp;<a class=\"trait\" href=\"mio/trait.Handler.html\" title=\"trait mio::Handler\">Handler</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/nightly/core/marker/trait.Sync.html\" title=\"trait core::marker::Sync\">Sync</a> for <a class=\"struct\" href=\"mio/struct.EventLoop.html\" title=\"struct mio::EventLoop\">EventLoop</a>&lt;H&gt;","impl&lt;M:&nbsp;<a class=\"trait\" href=\"https://doc.rust-lang.org/nightly/core/marker/trait.Send.html\" title=\"trait core::marker::Send\">Send</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/nightly/core/marker/trait.Sync.html\" title=\"trait core::marker::Sync\">Sync</a> for <a class=\"struct\" href=\"mio/struct.Sender.html\" title=\"struct mio::Sender\">Sender</a>&lt;M&gt;",];

            if (window.register_implementors) {
                window.register_implementors(implementors);
            } else {
                window.pending_implementors = implementors;
            }
        
})()
