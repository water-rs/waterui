import UIKit

/// The bench host's screen: which run is in progress, then one line
/// per run with its exit code (matching `done.json`), or the error
/// that kept the suite from starting.
final class StatusViewController: UIViewController {
    private let titleLabel = UILabel()
    private let detailLabel = UILabel()
    private let resultsStack = UIStackView()

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground

        titleLabel.font = .preferredFont(forTextStyle: .title1)
        detailLabel.font = .monospacedSystemFont(
            ofSize: UIFont.preferredFont(forTextStyle: .body).pointSize,
            weight: .regular
        )
        detailLabel.textColor = .secondaryLabel
        detailLabel.numberOfLines = 0

        resultsStack.axis = .vertical
        resultsStack.spacing = 8

        let stack = UIStackView(arrangedSubviews: [titleLabel, detailLabel, resultsStack])
        stack.axis = .vertical
        stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false

        let scrollView = UIScrollView()
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.addSubview(stack)
        view.addSubview(scrollView)

        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            scrollView.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor),
            stack.topAnchor.constraint(equalTo: scrollView.contentLayoutGuide.topAnchor, constant: 20),
            stack.leadingAnchor.constraint(equalTo: scrollView.contentLayoutGuide.leadingAnchor, constant: 20),
            stack.trailingAnchor.constraint(equalTo: scrollView.contentLayoutGuide.trailingAnchor, constant: -20),
            stack.bottomAnchor.constraint(equalTo: scrollView.contentLayoutGuide.bottomAnchor, constant: -20),
            stack.widthAnchor.constraint(equalTo: scrollView.frameLayoutGuide.widthAnchor, constant: -40),
        ])

        titleLabel.text = "CherenkovBench"
        detailLabel.text = "Preparing…"
    }

    private func reset(title: String, detail: String?) {
        titleLabel.text = title
        titleLabel.textColor = .label
        detailLabel.text = detail
        resultsStack.arrangedSubviews.forEach {
            resultsStack.removeArrangedSubview($0)
            $0.removeFromSuperview()
        }
    }
}

extension StatusViewController: BenchRunnerDelegate {
    func benchRunner(_: BenchRunner, didStartRun index: Int, of total: Int, args: [String]) {
        reset(title: "Running \(index + 1) of \(total)", detail: args.joined(separator: " "))
    }

    func benchRunner(_: BenchRunner, didFinishWithResults results: [BenchRunResult]) {
        reset(title: "Done", detail: nil)
        for result in results {
            let label = UILabel()
            label.font = .monospacedSystemFont(
                ofSize: UIFont.preferredFont(forTextStyle: .footnote).pointSize,
                weight: .regular
            )
            label.textColor = result.exitCode == 0 ? .label : .systemRed
            label.numberOfLines = 0
            label.text = "\(result.args.joined(separator: " ")) — exit \(result.exitCode)"
            resultsStack.addArrangedSubview(label)
        }
    }

    func benchRunner(_: BenchRunner, didFailWithError error: String) {
        reset(title: "Error", detail: error)
        titleLabel.textColor = .systemRed
    }
}
