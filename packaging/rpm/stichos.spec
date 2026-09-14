Name:           stichos
Version:        0.1.0
Release:        1%{?dist}
Summary:        Proof before publish — document inspection tool
License:        MIT OR Apache-2.0
BuildArch:      x86_64 aarch64

%description
stichos is a read-only tool for analyzing office documents.

%install
install -D -m755 stichos %{buildroot}/usr/bin/stichos

%files
/usr/bin/stichos

%posttrans
# No post-install actions needed
